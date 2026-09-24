//! Game launch orchestration: gamescope wrapping, env var injection, `OptiScaler` staging.
//!
//! Merges per-game `gamescope::Config` (profile) with global `VideoConfig` (video settings)
//! into a single `LaunchPlan` ready to `spawn()`.
//!
//! Priority, highest first:
//!
//! 1. The globally selected upscaling filter.
//! 2. The per-game `gamescope::Config`: resolution, frame limit, overlay.
//! 3. `VideoConfig.upscaling` base/target resolution, used when the profile
//!    specifies none of its own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::gamescope;
use crate::models::{
    FrameGenBackend, FrameGenSettings, GamescopeFilter, UpscalingSettings, WineFsrMode,
};
use crate::video_config::VideoConfig;

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
        // Audit LNCH-01: this used to return early unless power-profiles-daemon
        // reported `performance`, silently dropping Gamescope, Wine FSR,
        // vkBasalt and every frame-generation variable. A user who turned
        // Booster off lost their upscaler with no visible cause.
        //
        // Presentation-layer settings are not a CPU power policy. The two are
        // independent layers (docs/02-PERFORMANCE-AUTHORITY.md), so the gate is
        // gone: what the user configured is what gets applied.
        // Apply runtime harmony policy so enabled technologies do not conflict.
        let effective_video = Self::apply_harmony_policy(logical_game, video);
        let upscaling = &effective_video.upscaling;
        let frame_gen = &effective_video.frame_gen;

        // `steam -applaunch` starts the *client*, which then starts the game in
        // a separate process tree. Wrapping this command would put Gamescope
        // around the Steam client, not around the game, so the plan is left
        // alone here on purpose.
        //
        // That is not the whole answer, though. Audit LNCH-02: since Steam is
        // how most people launch games, leaving it at "we skip this case" made
        // the entire video pipeline inert in the common path. The mechanism
        // Steam provides is per-game launch options, so
        // [`LaunchPlan::as_steam_launch_options`] renders the same plan into
        // the string Steam understands, and `crate::steam` writes it.
        if Self::is_steam_applaunch_command(executable, executable_args) {
            tracing::info!(
                game = logical_game,
                "steam client launch: per-game settings belong in Steam's launch \
                 options, not around the client process"
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
        // The tri-state lives on the profile; when no profile is supplied the
        // global "enable Gamescope" toggle stands in for an explicit choice.
        let mode = if upscaling.gamescope_enabled {
            gamescope::Mode::Enabled
        } else {
            gamescope::Mode::Auto
        };
        let caps = crate::capabilities::Capabilities::detect().gamescope;
        let merged = Self::merge_gamescope_config(upscaling, gs_override);
        let decision = gamescope::decide(
            mode,
            &merged,
            caps.as_ref(),
            crate::hardware::Hardware::detect().session,
        );
        tracing::info!(
            target: "gamescope",
            game = logical_game,
            wrap = decision.use_gamescope,
            reason = %decision.reason,
            "gamescope decision"
        );
        if decision.use_gamescope {
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
    /// profile (resolution, framerate limit, `MangoHud` toggle); it is merged with the
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
    /// Two frame generators in series produce doubled and corrupted frames,
    /// not more frames:
    ///
    /// - `OptiScaler`/AFMF generate at the game's render level;
    /// - lsfg-vk generates at the Vulkan present level.
    ///
    /// One of the two has to be disabled.
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

    /// Check for known launch conflicts and emit `tracing::warn` entries.
    ///
    /// Called internally during `build()`; also publicly available for pre-launch
    /// UI validation (show dialogs before actually launching).
    pub fn check_conflicts(executable: &str, video: &VideoConfig) {
        Self::check_and_warn_conflicts(executable, video);
    }

    /// Render this plan as a Steam per-game launch options string.
    ///
    /// Steam substitutes `%command%` with the game's own command line, so the
    /// result is `VAR=value … gamescope … -- %command%`. Writing that into the
    /// game's launch options is what makes the plan apply to a Steam launch —
    /// the one path `build_with_args_for_game` deliberately cannot wrap.
    ///
    /// Returns `None` when the plan adds nothing, so a game with no settings is
    /// not given an empty wrapper.
    #[must_use]
    pub fn as_steam_launch_options(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();

        // Sorted so the same plan always renders the same string — otherwise
        // every save would look like a change to Steam and to the user.
        let mut keys: Vec<&String> = self.env.keys().collect();
        keys.sort();
        for key in keys {
            let value = &self.env[key];
            // Steam runs this through a shell and its own config format has no
            // escaping; anything needing quoting is dropped rather than risked.
            if value.contains([' ', '"', '\'', '\\', '\n']) {
                tracing::warn!(
                    target: "launch",
                    key,
                    "value needs shell quoting; omitted from Steam launch options"
                );
                continue;
            }
            parts.push(format!("{key}={value}"));
        }

        if self.program == "gamescope" {
            // Everything up to the `--` separator; the game command follows it,
            // and for Steam that is `%command%`.
            let sep = self.args.iter().position(|a| a == "--");
            let gs_args = sep.map_or(&self.args[..], |i| &self.args[..i]);
            parts.push("gamescope".to_owned());
            parts.extend(gs_args.iter().cloned());
        }

        if parts.is_empty() {
            return None;
        }
        Some(format!("{} -- %command%", parts.join(" ")))
    }

    /// Spawn the game as described by this plan.
    ///
    /// The child is placed in its own **process group**, so the whole tree can
    /// be signalled later with [`terminate`]. Games are routinely started
    /// through a wrapper — Lutris and many bundles ship a `run_game.sh` that
    /// execs the real binary as a grandchild — and without this, killing the
    /// returned handle kills only the wrapper and leaves the game running.
    ///
    /// That is not hypothetical: launching `SuperTuxKart` through this pipeline
    /// left `bin/supertuxkart` alive after the handle was killed and waited on.
    ///
    /// # Errors
    /// Returns an error if the binary is not found or the process fails to
    /// start.
    pub fn spawn(self) -> Result<std::process::Child> {
        use std::os::unix::process::CommandExt;

        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args);
        cmd.envs(&self.env);
        // SAFETY: `setpgid(0, 0)` is async-signal-safe and touches only the
        // calling process, which between fork and exec is the child alone.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        cmd.spawn()
            .with_context(|| format!("spawn '{}'", self.program))
    }
}

/// Ask a spawned game and everything it started to exit.
///
/// Sends `SIGTERM` to the child's whole process group — which
/// [`LaunchPlan::spawn`] created for exactly this purpose — then reaps the
/// direct child. Signalling only the child would leave a wrapper's grandchildren
/// running, which is the orphan this exists to prevent.
///
/// # Errors
/// Returns an error if the process could not be reaped.
pub fn terminate(child: &mut std::process::Child) -> Result<()> {
    let pid = i32::try_from(child.id()).context("child pid does not fit in pid_t")?;
    // SAFETY: a negative pid addresses the process group led by `pid`, which is
    // the group spawn() created. An already-exited group yields ESRCH, which is
    // not an error worth reporting here.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    child.wait().context("reap game process")?;
    Ok(())
}

// ── Gamescope args builder ────────────────────────────────────────────────────

impl LaunchPlan {
    /// Merge global upscaling settings with a per-game Gamescope override.
    ///
    /// Shared by the decision and the argument builder so the two can never
    /// disagree about what was configured.
    #[must_use]
    pub fn merge_gamescope_config(
        upscaling: &UpscalingSettings,
        gs_override: Option<&gamescope::Config>,
    ) -> gamescope::Config {
        let base = gs_override.cloned().unwrap_or_default();
        gamescope::Config {
            render_width: if upscaling.base_width > 0 {
                upscaling.base_width
            } else {
                base.render_width
            },
            render_height: if upscaling.base_height > 0 {
                upscaling.base_height
            } else {
                base.render_height
            },
            output_width: if upscaling.target_width > 0 {
                upscaling.target_width
            } else {
                base.output_width
            },
            output_height: if upscaling.target_height > 0 {
                upscaling.target_height
            } else {
                base.output_height
            },
            // `UpscalingSettings::gamescope_filter` defaults to `Fsr` rather
            // than to "none", so it says nothing about whether the user wants
            // upscaling — only which filter they would use if they did. Reading
            // it unconditionally made the Auto decision believe every profile
            // had requested FSR, and wrap every game.
            //
            // It is therefore honoured only when the user has actually turned
            // Gamescope upscaling on; otherwise the per-game override decides.
            filter: if upscaling.gamescope_enabled {
                match upscaling.gamescope_filter {
                    GamescopeFilter::Fsr => gamescope::Filter::Fsr,
                    GamescopeFilter::Nis => gamescope::Filter::Nis,
                    GamescopeFilter::Integer => gamescope::Filter::Pixel,
                }
            } else {
                base.filter
            },
            // Same reasoning as the filter above: `UpscalingSettings` carries a
            // sharpness even when Gamescope upscaling is off, and reading it
            // unconditionally silently overrode whatever the per-game profile
            // asked for. A profile requesting sharpness 4 was emitting 0.
            sharpness: if upscaling.gamescope_enabled {
                upscaling.clamped_sharpness()
            } else {
                base.sharpness
            },
            ..base
        }
    }
}

/// Build `("gamescope", argv)` from the global upscaling settings merged with a
/// per-game override.
///
/// All argument construction is delegated to [`gamescope::Config::to_args`],
/// which is the project's single builder and is capability-gated. This function
/// only decides *what* to ask for; the builder decides what this Gamescope
/// build can actually be given.
///
/// Resolution precedence, highest first:
/// 1. `UpscalingSettings.base_*` / `target_*` — an explicit render/output split;
/// 2. the per-game profile's render resolution;
/// 3. nothing, leaving Gamescope to follow the game.
fn build_gamescope_argv(
    executable: &str,
    executable_args: &[String],
    upscaling: &UpscalingSettings,
    gs_override: Option<&gamescope::Config>,
) -> (String, Vec<String>) {
    let caps = crate::capabilities::Capabilities::detect()
        .gamescope
        .unwrap_or_default();
    let cfg = LaunchPlan::merge_gamescope_config(upscaling, gs_override);

    let (argv, unsupported) = cfg.build_argv(&caps, executable, executable_args);
    for u in &unsupported {
        tracing::warn!(
            target: "gamescope",
            flag = %u.flag,
            effect = %u.effect,
            "installed gamescope does not support this option"
        );
    }
    ("gamescope".into(), argv)
}

// ── Environment variable builders ─────────────────────────────────────────────

/// Build the full set of persistent video-related environment variables for the
/// given configuration. Intended for writing into systemd user environment.d so
/// vars reach Steam-launched game processes that bypass our `spawn()`.
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

/// Copy `OptiScaler` DLLs from `source_dir` into `game_dir`.
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

/// Stage `OptiScaler` DLLs if enabled and source found. Logs on failure.
///
/// Silently does nothing if `OptiScaler` is disabled, backend is not `OptiScaler`,
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

/// Resolve the `OptiScaler` source directory from settings or well-known locations.
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
        // The removed `--fsr` flag must never appear; it aborts the launch.
        assert!(!plan.args.iter().any(|a| a == "--fsr"));
        if let Some(f_pos) = plan.args.iter().position(|a| a == "-F") {
            assert_eq!(plan.args[f_pos + 1], "fsr");
        }
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
        if let Some(f_pos) = plan.args.iter().position(|a| a == "-F") {
            assert_eq!(plan.args[f_pos + 1], "nis");
        }
        assert!(!plan.args.contains(&"--fsr".into()));
    }

    #[test]
    fn test_launch_plan_integer_scaling() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Integer;
        let plan = LaunchPlan::build("game", &video, None);
        if let Some(f_pos) = plan.args.iter().position(|a| a == "-F") {
            assert_eq!(plan.args[f_pos + 1], "pixel");
        }
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
    fn a_per_game_profile_keeps_its_own_filter_and_sharpness() {
        // The global UpscalingSettings carry a filter and a sharpness even
        // when Gamescope upscaling is off. Reading them unconditionally
        // overrode the profile: a profile asking for sharpness 4 emitted 0.
        let video = VideoConfig::default(); // gamescope_enabled = false
        let profile = gamescope::Config {
            filter: gamescope::Filter::Nis,
            sharpness: 4,
            ..gamescope::Config::default()
        };
        let merged = LaunchPlan::merge_gamescope_config(&video.upscaling, Some(&profile));
        assert_eq!(merged.filter, gamescope::Filter::Nis);
        assert_eq!(merged.sharpness, 4);
    }

    #[test]
    fn global_upscaling_settings_win_when_they_are_enabled() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Fsr;
        video.upscaling.gamescope_sharpness = 9;
        let profile = gamescope::Config {
            filter: gamescope::Filter::Nis,
            sharpness: 4,
            ..gamescope::Config::default()
        };
        let merged = LaunchPlan::merge_gamescope_config(&video.upscaling, Some(&profile));
        assert_eq!(merged.filter, gamescope::Filter::Fsr);
        assert_eq!(merged.sharpness, 9);
    }

    #[test]
    fn spawn_puts_the_child_in_its_own_process_group() {
        // Without this, killing the handle of a wrapper script leaves the game
        // it started running — verified against SuperTuxKart's run_game.sh.
        let plan = LaunchPlan {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 30 & wait".into()],
            env: HashMap::new(),
        };
        let mut child = plan.spawn().expect("spawn");
        let child_pid = i32::try_from(child.id()).unwrap();

        // SAFETY: reading the child's process group id.
        let group = unsafe { libc::getpgid(child_pid) };
        assert_eq!(group, child_pid, "child should lead its own process group");
        // And therefore not share ours.
        assert_ne!(group, unsafe { libc::getpgid(0) });

        terminate(&mut child).expect("terminate");
    }

    #[test]
    fn terminate_takes_down_the_whole_group() {
        // `sh -c 'sleep … & wait'` is the shape of a wrapper script: the thing
        // that matters is a grandchild.
        let plan = LaunchPlan {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 60 & echo $! > /dev/null; wait".into()],
            env: HashMap::new(),
        };
        let mut child = plan.spawn().expect("spawn");
        let pid = i32::try_from(child.id()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));

        terminate(&mut child).expect("terminate");
        std::thread::sleep(std::time::Duration::from_millis(300));

        // SAFETY: signal 0 only probes whether the group still exists.
        let alive = unsafe { libc::kill(-pid, 0) } == 0;
        assert!(!alive, "the process group should be gone");
    }

    #[test]
    fn steam_launch_options_render_env_and_gamescope() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.wine_fsr_mode = WineFsrMode::Quality;

        let plan = LaunchPlan::build("game", &video, None);
        let opts = plan.as_steam_launch_options().expect("plan adds settings");

        assert!(opts.ends_with(" -- %command%"), "got {opts}");
        assert!(opts.contains("WINE_FULLSCREEN_FSR=1"));
        assert!(opts.contains("gamescope"));
        // The separator appears exactly once, at the end.
        assert_eq!(opts.matches(" -- ").count(), 1);
    }

    #[test]
    fn steam_launch_options_are_stable_across_builds() {
        // An unstable ordering would make every save look like a change.
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.vkbasalt_enabled = true;
        let a = LaunchPlan::build("game", &video, None).as_steam_launch_options();
        let b = LaunchPlan::build("game", &video, None).as_steam_launch_options();
        assert_eq!(a, b);
        assert!(a.is_some());
    }

    #[test]
    fn a_plan_that_adds_nothing_produces_no_launch_options() {
        let video = VideoConfig::default();
        let plan = LaunchPlan::build("game", &video, None);
        assert_eq!(plan.as_steam_launch_options(), None);
    }

    #[test]
    fn values_needing_shell_quoting_are_omitted_not_mangled() {
        let mut plan = LaunchPlan::build("game", &VideoConfig::default(), None);
        plan.env.insert("SAFE".into(), "1".into());
        plan.env.insert("RISKY".into(), "has spaces".into());
        let opts = plan.as_steam_launch_options().unwrap();
        assert!(opts.contains("SAFE=1"));
        assert!(!opts.contains("RISKY"));
    }

    #[test]
    fn the_steam_client_command_is_never_wrapped() {
        // Wrapping `steam -applaunch` would put Gamescope around the client.
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let args = vec!["-applaunch".to_string(), "1808500".to_string()];
        let plan = LaunchPlan::build_with_args("steam", &args, &video, None);
        assert_eq!(plan.program, "steam");
        assert_eq!(plan.args, args);
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
