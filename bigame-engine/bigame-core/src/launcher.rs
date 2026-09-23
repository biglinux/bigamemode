//! Game launch orchestration: gamescope wrapping, env var injection, OptiScaler staging.
//!
//! Merges per-game `gamescope::Config` (profile) with global `VideoConfig` (video settings)
//! into a single `LaunchPlan` ready to `spawn()`.
//!
//! Priority (highest → lowest):
//! - `VideoConfig.upscaling.gamescope_filter` (new UIspecific to each filter)
//! - Per-game `gamescope::Config` resolution / framerate / mangohud
//! - `VideoConfig.upscaling.base_*/target_*` resolution (falls back when profile has none)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::gamescope;
use crate::models::{
    FrameGenBackend, FrameGenSettings, GamescopeFilter, UpscalingSettings, WineFsrMode,
};
use crate::video_config::VideoConfig;

/// Env override used by tests and diagnostics.
///
/// - `BIGAME_TURBO_MODE=on`  => force turbo active
/// - `BIGAME_TURBO_MODE=off` => force turbo inactive
const TURBO_OVERRIDE_ENV: &str = "BIGAME_TURBO_MODE";

// ── LaunchPlan ────────────────────────────────────────────────────────────────

/// Fully resolved plan to launch a game with all BiGame-mode video settings applied.
#[derive(Debug, Clone)]
pub struct LaunchPlan {
    /// Top-level executable (`"gamescope"` or game path).
    pub program: String,
    /// Command-line arguments passed to `program`.
    pub args: Vec<String>,
    /// Environment variables to inject alongside the parent environment.
    pub env: HashMap<String, String>,
}

impl LaunchPlan {
    /// Build a launch plan for `executable` with explicit executable args.
    #[must_use]
    pub fn build_with_args(
        executable: &str,
        executable_args: &[String],
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Self {
        Self::build_with_args_for_game(executable, executable_args, executable, video, gs_override)
    }

    /// Build a launch plan and evaluate policy against a logical game id.
    ///
    /// `logical_game` should be the profile/process identifier representing the real game
    /// (for example, Steam `installdir`). It can differ from `executable` (for example,
    /// `executable="steam"` with `-applaunch`).
    #[must_use]
    pub fn build_with_args_for_game(
        executable: &str,
        executable_args: &[String],
        logical_game: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Self {
        // Hard gate: advanced video enhancements only apply with Turbo mode enabled.
        if !Self::is_turbo_mode_active() {
            tracing::info!(
                game = executable,
                "Turbo mode inactive -> launching without Gamescope/env injections"
            );
            return Self {
                program: executable.to_string(),
                args: executable_args.to_vec(),
                env: HashMap::new(),
            };
        }

        // Apply runtime harmony policy so enabled technologies do not conflict.
        let effective_video = Self::apply_harmony_policy(logical_game, video);
        let upscaling = &effective_video.upscaling;
        let frame_gen = &effective_video.frame_gen;

        let is_steam_applaunch = Self::is_steam_applaunch_command(executable, executable_args);
        if is_steam_applaunch {
            tracing::info!(
                game = executable,
                args = ?executable_args,
                "steam launch detected -> bypassing video env injections for stability"
            );
            return Self {
                program: executable.to_string(),
                args: executable_args.to_vec(),
                env: HashMap::new(),
            };
        }

        // ── Environment variables ─────────────────────────────────────────────
        let mut env = HashMap::new();
        Self::check_and_warn_conflicts(logical_game, &effective_video);

        collect_upscaling_env(upscaling, &mut env);
        collect_framegen_env(frame_gen, &mut env);

        // ── Decide program + args ─────────────────────────────────────────────
        let use_gamescope = upscaling.gamescope_enabled || gs_override.is_some();
        if use_gamescope {
            let (program, args) =
                build_gamescope_argv(executable, executable_args, upscaling, gs_override);
            Self { program, args, env }
        } else {
            Self {
                program: executable.to_string(),
                args: executable_args.to_vec(),
                env,
            }
        }
    }

    /// Build a launch plan for `executable`.
    ///
    /// `video` is the global video config. `gs_override` is the per-game gamescope
    /// profile (resolution, framerate limit, MangoHud toggle); it is merged with the
    /// global upscaling filter chosen in `video`.
    #[must_use]
    pub fn build(
        executable: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Self {
        Self::build_with_args(executable, &[], video, gs_override)
    }

    /// Apply conflict-resolution policy and return an effective launch config.
    ///
    /// Policy goals:
    /// - Keep user intent whenever possible.
    /// - Prevent double frame-generation pipelines at launch time.
    /// - Auto-heal common conflicts instead of only warning.
    fn apply_harmony_policy(executable: &str, video: &VideoConfig) -> VideoConfig {
        let mut effective = video.clone();

        if !effective.frame_gen.enabled {
            return effective;
        }

        match effective.frame_gen.backend {
            FrameGenBackend::OptiScaler | FrameGenBackend::Afmf => {
                // If lsfg-vk is active for this game, disable it for this game automatically.
                if crate::fg::is_active_for_game(executable) {
                    match crate::fg::disable_for_game(executable) {
                        Ok(()) => tracing::info!(
                            game = executable,
                            backend = ?effective.frame_gen.backend,
                            "harmony policy: disabled lsfg-vk for this game to avoid double frame generation"
                        ),
                        Err(e) => tracing::warn!(
                            game = executable,
                            backend = ?effective.frame_gen.backend,
                            error = %e,
                            "harmony policy: failed to disable conflicting lsfg-vk profile"
                        ),
                    }
                }
            }
            FrameGenBackend::LsfgVk => {
                if !crate::fg::is_lossless_dll_ready() {
                    effective.frame_gen.enabled = false;
                    if let Err(e) = crate::fg::disable_all_profiles() {
                        tracing::warn!(
                            game = executable,
                            error = %e,
                            "harmony policy: failed to disable lsfg profiles after missing Lossless.dll"
                        );
                    }
                    tracing::warn!(
                        game = executable,
                        "harmony policy: LSFG-VK disabled because Lossless.dll path is missing/invalid"
                    );
                    return effective;
                }

                // lsfg-vk backend: keep only lsfg path and neutralize other FG toggles.
                if effective.frame_gen.optiscaler_enabled {
                    effective.frame_gen.optiscaler_enabled = false;
                    tracing::info!(
                        game = executable,
                        "harmony policy: disabled OptiScaler staging because backend=lsfg-vk"
                    );
                }
                if effective.frame_gen.afmf_experimental_enabled {
                    effective.frame_gen.afmf_experimental_enabled = false;
                    tracing::info!(
                        game = executable,
                        "harmony policy: disabled AFMF experimental vars because backend=lsfg-vk"
                    );
                }
            }
            FrameGenBackend::None => {}
        }

        effective
    }

    // ── Conflict detection ─────────────────────────────────────────────────────

    /// Emit structured warnings for any known frame generation conflicts.
    ///
    /// Conflicts occur when two frame generation technologies are active simultaneously:
    /// - OptiScaler/AFMF generates frames at the game render level
    /// - lsfg-vk generates frames at the Vulkan present level
    /// Running both causes doubled/corrupted frames. Users must disable one.
    fn check_and_warn_conflicts(executable: &str, video: &VideoConfig) {
        if !video.frame_gen.enabled {
            return;
        }
        match video.frame_gen.backend {
            FrameGenBackend::OptiScaler | FrameGenBackend::Afmf => {
                // Conflict: OptiScaler/AFMF + lsfg-vk active for same game
                if crate::fg::is_active_for_game(executable) {
                    tracing::warn!(
                        game = executable,
                        backend = ?video.frame_gen.backend,
                        "FRAME GEN CONFLICT: {} has lsfg-vk FG enabled AND {:?} selected — \
                         disable one to avoid rendering artifacts",
                        executable,
                        video.frame_gen.backend,
                    );
                }
            }
            FrameGenBackend::LsfgVk => {
                // Conflict: lsfg-vk backend but OptiScaler staging also enabled
                if video.frame_gen.optiscaler_enabled {
                    tracing::warn!(
                        game = executable,
                        "FRAME GEN CONFLICT: lsfg-vk backend + OptiScaler staging both active for '{}' — \
                     disable 'Stage OptiScaler DLLs' to avoid conflicts",
                        executable,
                    );
                }
            }
            FrameGenBackend::None => {}
        }
    }

    #[must_use]
    fn is_steam_applaunch_command(executable: &str, executable_args: &[String]) -> bool {
        if !executable.eq_ignore_ascii_case("steam") {
            return false;
        }

        executable_args
            .iter()
            .any(|arg| arg.eq_ignore_ascii_case("-applaunch"))
    }

    /// Runtime turbo mode gate for video enhancements.
    ///
    /// Reads `BIGAME_TURBO_MODE` first for deterministic tests, then falls back to
    /// PowerProfiles D-Bus (`performance` means Turbo active).
    #[must_use]
    fn is_turbo_mode_active() -> bool {
        if let Ok(override_mode) = std::env::var(TURBO_OVERRIDE_ENV) {
            let mode = override_mode.trim().to_ascii_lowercase();
            if mode == "on" || mode == "1" || mode == "true" {
                return true;
            }
            if mode == "off" || mode == "0" || mode == "false" {
                return false;
            }
        }

        crate::dbus::power_profile_get()
            .map(|p| p.eq_ignore_ascii_case("performance"))
            .unwrap_or(false)
    }

    /// Check for known launch conflicts and emit `tracing::warn` entries.
    ///
    /// Called internally during `build()`; also publicly available for pre-launch
    /// UI validation (show dialogs before actually launching).
    pub fn check_conflicts(executable: &str, video: &VideoConfig) {
        Self::check_and_warn_conflicts(executable, video);
    }

    /// Spawn the game as described by this plan.
    ///
    /// # Errors
    /// Returns error if the binary is not found or the process fails to start.
    pub fn spawn(self) -> Result<std::process::Child> {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args);
        cmd.envs(&self.env);
        cmd.spawn()
            .with_context(|| format!("spawn '{}'", self.program))
    }
}

// ── Gamescope args builder ────────────────────────────────────────────────────

/// Build `("gamescope", args_vec)` with merged profile + upscaling settings.
///
/// Resolution priority (highest → lowest):
/// 1. `UpscalingSettings.base_*` / `target_*` (non-zero) — enables proper render/output split
/// 2. Per-game `gamescope::Config.width/height` — output resolution from profile
/// 3. Nothing (gamescope uses game-reported defaults)
fn build_gamescope_argv(
    executable: &str,
    executable_args: &[String],
    upscaling: &UpscalingSettings,
    gs_override: Option<&gamescope::Config>,
) -> (String, Vec<String>) {
    let mut args: Vec<String> = Vec::new();

    // ── Render resolution (game draws at this) ─────────────────────────────
    // Use upscaling.base_* if set; otherwise fall back to profile's width/height
    let render_w = if upscaling.base_width > 0 {
        Some(upscaling.base_width)
    } else {
        gs_override.filter(|c| c.width > 0).map(|c| c.width)
    };
    let render_h = if upscaling.base_height > 0 {
        Some(upscaling.base_height)
    } else {
        gs_override.filter(|c| c.height > 0).map(|c| c.height)
    };

    // ── Display output resolution (upscaled to this) ───────────────────────
    let target_w = if upscaling.target_width > 0 {
        Some(upscaling.target_width)
    } else {
        None
    };
    let target_h = if upscaling.target_height > 0 {
        Some(upscaling.target_height)
    } else {
        None
    };

    if let (Some(w), Some(h)) = (render_w, render_h) {
        args.extend(["-w".into(), w.to_string(), "-h".into(), h.to_string()]);
    }
    if let (Some(w), Some(h)) = (target_w, target_h) {
        args.extend(["-W".into(), w.to_string(), "-H".into(), h.to_string()]);
    }

    // ── Framerate limit + MangoHud from profile ────────────────────────────
    if let Some(cfg) = gs_override {
        if cfg.framerate_limit > 0 {
            args.extend(["-r".into(), cfg.framerate_limit.to_string()]);
        }
        if cfg.mangohud {
            args.push("--mangoapp".into());
        }
    }

    // ── Upscaling filter — gamescope ≥3.14 uses -F/--filter; old --fsr/--nis
    //    were removed (they share prefix with --fsr-sharpness → parse collision)
    match upscaling.gamescope_filter {
        GamescopeFilter::Fsr => {
            args.extend(["-F".into(), "fsr".into()]);
            args.push("--fsr-sharpness".into());
            args.push(upscaling.clamped_sharpness().to_string());
        }
        GamescopeFilter::Nis => {
            args.extend(["-F".into(), "nis".into()]);
            args.push("--fsr-sharpness".into());
            args.push(upscaling.clamped_sharpness().to_string());
        }
        GamescopeFilter::Integer => {
            args.extend(["-F".into(), "pixel".into()]);
        }
    }

    // Separator between gamescope args and game command
    args.push("--".into());
    args.push(executable.to_string());
    args.extend(executable_args.iter().cloned());

    ("gamescope".into(), args)
}

// ── Environment variable builders ─────────────────────────────────────────────

/// Build the full set of persistent video-related environment variables for the
/// given configuration. Intended for writing into systemd user environment.d so
/// vars reach Steam-launched game processes that bypass our spawn().
#[must_use]
pub fn build_persistent_env(video: &crate::video_config::VideoConfig) -> HashMap<String, String> {
    let mut env = HashMap::new();
    collect_upscaling_env(&video.upscaling, &mut env);
    collect_framegen_env(&video.frame_gen, &mut env);
    env
}

/// Insert Wine FSR env vars if enabled.
fn collect_upscaling_env(upscaling: &UpscalingSettings, env: &mut HashMap<String, String>) {
    if upscaling.wine_fsr_enabled {
        env.insert("WINE_FULLSCREEN_FSR".into(), "1".into());
        let mode = match upscaling.wine_fsr_mode {
            WineFsrMode::Performance => "performance",
            WineFsrMode::Balanced => "balanced",
            WineFsrMode::Quality => "quality",
            WineFsrMode::Ultra => "ultra",
        };
        env.insert("WINE_FULLSCREEN_FSR_MODE".into(), mode.into());
    }

    if upscaling.vkbasalt_enabled {
        env.insert("ENABLE_VKBASALT".into(), "1".into());
        if let Some(path) = &upscaling.vkbasalt_config_path {
            if !path.is_empty() && std::path::Path::new(path).is_file() {
                env.insert("VKBASALT_CONFIG_FILE".into(), path.clone());
            }
        }
    }
}

/// Insert frame generation env vars if enabled.
fn collect_framegen_env(fg: &FrameGenSettings, env: &mut HashMap<String, String>) {
    if !fg.enabled {
        return;
    }
    if fg.backend == FrameGenBackend::Afmf && fg.afmf_experimental_enabled {
        // Override string format: "KEY=VALUE" or just "RADV_PERFTEST=afmf" fallback
        let override_str = fg
            .afmf_env_override
            .as_deref()
            .unwrap_or("RADV_PERFTEST=afmf");
        if let Some((key, val)) = override_str.split_once('=') {
            env.insert(key.to_string(), val.to_string());
        } else {
            env.insert("RADV_PERFTEST".into(), "afmf".into());
        }
    }
}

// ── OptiScaler DLL staging ─────────────────────────────────────────────────────

/// Copy OptiScaler DLLs from `source_dir` into `game_dir`.
///
/// Files copied (if present): `dxgi.dll`, `nvngx.dll`, `_nvngx.dll`, `OptiScaler.ini`.
/// Missing files in source are silently skipped.
///
/// # Errors
/// Returns `Err` if `game_dir` cannot be created or any present DLL cannot be copied.
pub fn stage_optiscaler_dlls(source_dir: &Path, game_dir: &Path) -> Result<()> {
    const DLLS: &[&str] = &["dxgi.dll", "nvngx.dll", "_nvngx.dll", "OptiScaler.ini"];

    std::fs::create_dir_all(game_dir)
        .with_context(|| format!("create game dir: {}", game_dir.display()))?;

    for name in DLLS {
        let src = source_dir.join(name);
        if !src.exists() {
            continue; // Optional — skip missing files
        }
        let dst = game_dir.join(name);
        std::fs::copy(&src, &dst)
            .with_context(|| format!("copy {name}: {} → {}", src.display(), dst.display()))?;
    }
    Ok(())
}

/// Stage OptiScaler DLLs if enabled and source found. Logs on failure.
///
/// Silently does nothing if OptiScaler is disabled, backend is not OptiScaler,
/// source dir is not found, or `game_dir` is `None`.
pub fn maybe_stage_optiscaler(fg: &FrameGenSettings, game_dir: Option<&Path>) {
    if !fg.enabled || !fg.optiscaler_enabled || fg.backend != FrameGenBackend::OptiScaler {
        return;
    }
    let Some(game_dir) = game_dir else {
        tracing::debug!("OptiScaler staging skipped: game install path unknown");
        return;
    };
    let Some(src) = resolve_optiscaler_source(fg) else {
        tracing::warn!(
            "OptiScaler staging skipped: source dir not found (set it in Video → Frame Generation → OptiScaler Source Directory)"
        );
        return;
    };
    if let Err(e) = stage_optiscaler_dlls(&src, game_dir) {
        tracing::warn!("OptiScaler staging failed: {e:#}");
    } else {
        tracing::info!(
            "OptiScaler staged from {} → {}",
            src.display(),
            game_dir.display()
        );
    }
}

/// Resolve the OptiScaler source directory from settings or well-known locations.
///
/// Returns `None` if no valid directory is found.
#[must_use]
pub fn resolve_optiscaler_source(fg: &FrameGenSettings) -> Option<PathBuf> {
    // Configured path takes priority
    if let Some(dir) = &fg.optiscaler_source_dir {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    // Well-known fallback install locations
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        PathBuf::from(&home).join(".local/share/optiscaler"),
        PathBuf::from("/usr/share/optiscaler"),
        PathBuf::from("/usr/local/share/optiscaler"),
        PathBuf::from("/opt/optiscaler"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FrameGenBackend, GamescopeFilter, WineFsrMode};

    #[test]
    fn test_launch_plan_no_gamescope_returns_exe() {
        let video = VideoConfig::default(); // gamescope_enabled = false
        let plan = LaunchPlan::build("myapp", &video, None);
        assert_eq!(plan.program, "myapp");
        assert!(plan.args.is_empty());
        assert!(plan.env.is_empty());
    }

    #[test]
    fn test_launch_plan_gamescope_enabled_wraps() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Fsr;
        video.upscaling.gamescope_sharpness = 5;
        let plan = LaunchPlan::build("myapp", &video, None);
        assert_eq!(plan.program, "gamescope");
        // gamescope ≥3.14: -F fsr replaces old --fsr bool flag
        let f_pos = plan.args.iter().position(|a| a == "-F").unwrap();
        assert_eq!(plan.args[f_pos + 1], "fsr");
        assert!(plan.args.contains(&"--fsr-sharpness".into()));
        assert!(plan.args.contains(&"5".into()));
        // Separator before exe
        let sep_pos = plan.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(plan.args[sep_pos + 1], "myapp");
    }

    #[test]
    fn test_launch_plan_nis_filter() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Nis;
        let plan = LaunchPlan::build("game", &video, None);
        let f_pos = plan.args.iter().position(|a| a == "-F").unwrap();
        assert_eq!(plan.args[f_pos + 1], "nis");
        assert!(!plan.args.contains(&"--fsr".into()));
    }

    #[test]
    fn test_launch_plan_integer_scaling() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Integer;
        let plan = LaunchPlan::build("game", &video, None);
        let f_pos = plan.args.iter().position(|a| a == "-F").unwrap();
        assert_eq!(plan.args[f_pos + 1], "pixel");
    }

    #[test]
    fn test_launch_plan_wine_fsr_env() {
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.wine_fsr_mode = WineFsrMode::Ultra;
        let plan = LaunchPlan::build("game", &video, None);
        assert_eq!(plan.env.get("WINE_FULLSCREEN_FSR").unwrap(), "1");
        assert_eq!(plan.env.get("WINE_FULLSCREEN_FSR_MODE").unwrap(), "ultra");
    }

    #[test]
    fn test_launch_plan_vkbasalt_env() {
        // Create a real temp file so VKBASALT_CONFIG_FILE is included.
        let tmp = std::env::temp_dir().join("bigame_test_vkBasalt.conf");
        std::fs::write(&tmp, "").expect("write tmp vkbasalt config");

        let mut video = VideoConfig::default();
        video.upscaling.vkbasalt_enabled = true;
        video.upscaling.vkbasalt_config_path = Some(tmp.to_string_lossy().into_owned());
        let plan = LaunchPlan::build("game", &video, None);
        assert_eq!(plan.env.get("ENABLE_VKBASALT").unwrap(), "1");
        assert_eq!(
            plan.env.get("VKBASALT_CONFIG_FILE").unwrap(),
            &tmp.to_string_lossy().into_owned()
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_launch_plan_afmf_env() {
        let mut video = VideoConfig::default();
        video.frame_gen.enabled = true;
        video.frame_gen.backend = FrameGenBackend::Afmf;
        video.frame_gen.afmf_experimental_enabled = true;
        let plan = LaunchPlan::build("game", &video, None);
        assert_eq!(plan.env.get("RADV_PERFTEST").unwrap(), "afmf");
    }

    #[test]
    fn test_launch_plan_afmf_custom_env_override() {
        let mut video = VideoConfig::default();
        video.frame_gen.enabled = true;
        video.frame_gen.backend = FrameGenBackend::Afmf;
        video.frame_gen.afmf_experimental_enabled = true;
        video.frame_gen.afmf_env_override = Some("CUSTOM_VAR=value123".into());
        let plan = LaunchPlan::build("game", &video, None);
        assert_eq!(plan.env.get("CUSTOM_VAR").unwrap(), "value123");
        assert!(!plan.env.contains_key("RADV_PERFTEST"));
    }

    #[test]
    fn test_launch_plan_framegen_disabled_no_env() {
        let mut video = VideoConfig::default();
        video.frame_gen.enabled = false;
        video.frame_gen.afmf_experimental_enabled = true; // should not fire if disabled
        let plan = LaunchPlan::build("game", &video, None);
        assert!(!plan.env.contains_key("RADV_PERFTEST"));
    }

    #[test]
    fn test_launch_plan_resolution_from_upscaling() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.base_width = 1280;
        video.upscaling.base_height = 720;
        video.upscaling.target_width = 1920;
        video.upscaling.target_height = 1080;
        let plan = LaunchPlan::build("game", &video, None);
        let args = &plan.args;
        let w_pos = args.iter().position(|a| a == "-w").unwrap();
        assert_eq!(args[w_pos + 1], "1280");
        let h_pos = args.iter().position(|a| a == "-h").unwrap();
        assert_eq!(args[h_pos + 1], "720");
        let bw_pos = args.iter().position(|a| a == "-W").unwrap();
        assert_eq!(args[bw_pos + 1], "1920");
    }

    #[test]
    fn test_launch_plan_steam_applaunch_skips_gamescope_wrapper() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Fsr;

        let args = vec!["-applaunch".to_string(), "750920".to_string()];
        let plan = LaunchPlan::build_with_args("steam", &args, &video, None);

        assert_eq!(plan.program, "steam");
        assert_eq!(plan.args, args);
        assert!(
            !plan.env.contains_key("WINE_FULLSCREEN_FSR")
                && !plan.env.contains_key("ENABLE_VKBASALT")
        );
    }

    #[test]
    fn test_stage_optiscaler_dlls_copies_existing() {
        let src_dir = std::env::temp_dir().join(format!("optiscaler_src_{}", std::process::id()));
        let dst_dir = std::env::temp_dir().join(format!("optiscaler_dst_{}", std::process::id()));
        std::fs::create_dir_all(&src_dir).unwrap();
        // Create a fake DLL
        std::fs::write(src_dir.join("dxgi.dll"), b"FAKE").unwrap();

        stage_optiscaler_dlls(&src_dir, &dst_dir).unwrap();
        assert!(dst_dir.join("dxgi.dll").exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&dst_dir);
    }
}
