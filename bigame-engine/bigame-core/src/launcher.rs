//! Game launch orchestration: gamescope wrapping and env var injection, with
//! the Harmony Policy keeping technologies that do the same job from stacking.
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

use anyhow::{Context, Result};

use crate::gamescope;
use crate::models::{FrameGenBackend, GamescopeFilter, UpscalingSettings, WineFsrMode};
use crate::video_config::VideoConfig;

// ── LaunchPlan ────────────────────────────────────────────────────────────────

/// What the machine offers a launch: Gamescope (and what it accepts), and a
/// graphical session for it to nest in.
///
/// Detected for a real launch. Tests describe it instead: a package built on
/// a server, in a chroot or over ssh has neither, and the plan a test checks
/// must not depend on the machine that happens to run it.
#[derive(Debug, Clone)]
struct Host {
    gamescope: Option<crate::capabilities::GamescopeCaps>,
    session: crate::hardware::Session,
}

impl Host {
    fn detect() -> Self {
        Self {
            gamescope: crate::capabilities::Capabilities::detect().gamescope,
            session: crate::hardware::Hardware::detect().session,
        }
    }
}

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
    /// `logical_game` is the process name the game's profile is keyed on. It can differ
    /// from `executable` (for example, `executable="steam"` with `-applaunch`).
    #[must_use]
    pub fn build_with_args_for_game(
        executable: &str,
        executable_args: &[String],
        logical_game: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Self {
        Self::build_on(
            &Host::detect(),
            executable,
            executable_args,
            logical_game,
            video,
            gs_override,
        )
    }

    /// [`Self::build_with_args_for_game`] on a given machine rather than this
    /// one.
    fn build_on(
        host: &Host,
        executable: &str,
        executable_args: &[String],
        logical_game: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Self {
        // Presentation-layer settings (Gamescope, Wine FSR, vkBasalt, frame
        // generation) are not a CPU power policy and do not depend on the power
        // profile: what the user configured is applied whatever Booster or
        // Turbo are doing. The harmony policy keeps enabled technologies from
        // conflicting.
        let mut effective_video = Self::apply_harmony_policy(logical_game, video);
        // A game BiGame-mode installed OptiScaler into already upscales;
        // Gamescope and Wine FSR would be second upscalers.
        let disables = crate::graphics::launch_disables(
            &crate::graphics::state_dir(),
            &crate::game_settings::dir(),
            logical_game,
        );
        let gs_local = Self::apply_graphics_disables(
            logical_game,
            &disables,
            &mut effective_video,
            gs_override,
        );
        let gs_override = gs_local.as_ref();
        let upscaling = &effective_video.upscaling;

        // `steam -applaunch` starts the *client*, which then starts the game in
        // a separate process tree. Wrapping this command would put Gamescope
        // around the Steam client, not around the game, so the plan is left
        // alone here on purpose: a game started through the Steam client gets
        // none of these video settings. falcond's per-game profile still
        // applies to it, since falcond matches the game's process.
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
        if disables.contains(&crate::graphics::rules::Tech::LsfgVk) {
            // The lsfg-vk layer's own off switch (its `disable_environment`).
            env.insert("DISABLE_LSFG".to_owned(), "1".to_owned());
        }

        // ── Decide program + args ─────────────────────────────────────────────
        // The tri-state lives on the profile; when no profile is supplied the
        // global "enable Gamescope" toggle stands in for an explicit choice.
        let mode = if upscaling.gamescope_enabled {
            gamescope::Mode::Enabled
        } else {
            gamescope::Mode::Auto
        };
        let merged = Self::merge_gamescope_config(upscaling, gs_override);
        let decision = gamescope::decide(mode, &merged, host.gamescope.as_ref(), host.session);
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

    /// Apply conflict-resolution policy and return an effective launch config.
    ///
    /// Only lsfg-vk remains a global frame-generation backend; per-game
    /// frame generation through `OptiScaler` is a game's AI Graphics and is
    /// reconciled by [`Self::apply_graphics_disables`]. lsfg-vk without its
    /// `Lossless.dll` would load and do nothing, so it is switched off.
    fn apply_harmony_policy(executable: &str, video: &VideoConfig) -> VideoConfig {
        let mut effective = video.clone();
        if effective.frame_gen.enabled
            && effective.frame_gen.backend == FrameGenBackend::LsfgVk
            && !crate::fg::is_lossless_dll_ready()
        {
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
        }
        effective
    }

    /// Turn off, for this launch only, what the game's AI Graphics makes a
    /// second upscaler (see `graphics::rules`). The global settings are not
    /// changed; the returned Gamescope config replaces the per-game one when
    /// its render size had to go.
    fn apply_graphics_disables(
        game: &str,
        disables: &[crate::graphics::rules::Tech],
        video: &mut VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Option<gamescope::Config> {
        use crate::graphics::rules::Tech;
        let mut gs = gs_override.cloned();
        if disables.contains(&Tech::WineFsr) && video.upscaling.wine_fsr_enabled {
            video.upscaling.wine_fsr_enabled = false;
            tracing::info!(target: "graphics", game, "harmony: Wine FSR off for this launch — OptiScaler already upscales");
        }
        if disables.contains(&Tech::GamescopeUpscaling) {
            let scaled =
                video.upscaling.base_width > 0 || gs.as_ref().is_some_and(|g| g.render_width > 0);
            // Gamescope upscales only when it renders below its output size;
            // without a render size the game renders at the output, and
            // Gamescope still wraps it if the user wanted that for anything else.
            video.upscaling.base_width = 0;
            video.upscaling.base_height = 0;
            if let Some(g) = gs.as_mut() {
                g.render_width = 0;
                g.render_height = 0;
            }
            if scaled {
                tracing::info!(target: "graphics", game, "harmony: Gamescope upscaling off for this launch — OptiScaler already upscales");
            }
        }
        gs
    }

    // ── Conflict detection ─────────────────────────────────────────────────────

    /// Emit structured warnings for launch conflicts.
    ///
    /// lsfg-vk selected but its `Lossless.dll` missing is the one left at the
    /// global level; per-game conflicts are reported by the game's AI
    /// Graphics plan.
    fn check_and_warn_conflicts(executable: &str, video: &VideoConfig) {
        if video.frame_gen.enabled
            && video.frame_gen.backend == FrameGenBackend::LsfgVk
            && !crate::fg::is_lossless_dll_ready()
        {
            tracing::warn!(
                game = executable,
                "lsfg-vk is selected but its Lossless.dll is not configured"
            );
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

    /// Spawn the game as described by this plan.
    ///
    /// The child is placed in its own **process group**, so the whole tree can
    /// be signalled later with [`terminate`]. Games are routinely started
    /// through a wrapper — Lutris and many bundles ship a `run_game.sh` that
    /// execs the real binary as a grandchild — and without this, killing the
    /// returned handle kills only the wrapper and leaves the game running.
    ///
    /// # Errors
    /// Returns an error if the binary is not found or the process fails to
    /// start.
    pub fn spawn(self) -> Result<std::process::Child> {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args);
        cmd.envs(&self.env);
        in_own_process_group(&mut cmd);
        cmd.spawn()
            .with_context(|| format!("spawn '{}'", self.program))
    }
}

/// Start `cmd` as the leader of a new process group, so [`terminate`] can
/// reach everything it starts — a wrapper script's game included.
pub fn in_own_process_group(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
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
}

/// Ask a spawned game and everything it started to exit.
///
/// Signals the child's whole process group — created by
/// [`in_own_process_group`] for exactly this purpose — so a wrapper's
/// grandchildren go too. `SIGTERM` first; a group still there after a few
/// seconds gets `SIGKILL`, so a game that ignores the request cannot hang the
/// caller.
///
/// # Errors
/// Returns an error if the process could not be reaped.
pub fn terminate(child: &mut std::process::Child) -> Result<()> {
    let pid = i32::try_from(child.id()).context("child pid does not fit in pid_t")?;
    let signal_group = |signal| {
        // SAFETY: a negative pid addresses the process group led by `pid`.
        // An already-exited group yields ESRCH, which is not worth reporting.
        unsafe {
            libc::kill(-pid, signal);
        }
    };
    signal_group(libc::SIGTERM);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if child.try_wait().context("reap game process")?.is_some() {
            // The leader is gone; anything it left behind is not.
            signal_group(libc::SIGKILL);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    signal_group(libc::SIGKILL);
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
            // upscaling — only which filter they would use if they did. Read
            // unconditionally, it would make every profile look as if it had
            // requested FSR, and the Auto decision would wrap every game.
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
            // sharpness even when Gamescope upscaling is off, and read
            // unconditionally it would override whatever the per-game profile
            // asks for.
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
    env
}

/// Insert the Wine FSR and vkBasalt variables for whichever is enabled.
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{GamescopeFilter, WineFsrMode};

    /// A desktop with Gamescope installed, whatever runs the tests.
    fn desktop() -> Host {
        Host {
            gamescope: Some(crate::capabilities::GamescopeCaps {
                version: None,
                flags: Vec::new(),
            }),
            session: crate::hardware::Session::Wayland,
        }
    }

    fn build(exe: &str, video: &VideoConfig, gs: Option<&gamescope::Config>) -> LaunchPlan {
        LaunchPlan::build_on(&desktop(), exe, &[], exe, video, gs)
    }

    fn build_with_args(
        exe: &str,
        args: &[String],
        video: &VideoConfig,
        gs: Option<&gamescope::Config>,
    ) -> LaunchPlan {
        LaunchPlan::build_on(&desktop(), exe, args, exe, video, gs)
    }

    #[test]
    fn test_launch_plan_no_gamescope_returns_exe() {
        let video = VideoConfig::default(); // gamescope_enabled = false
        let plan = build("myapp", &video, None);
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
        let plan = build("myapp", &video, None);
        assert_eq!(plan.program, "gamescope");
        // The removed `--fsr` flag must never appear; it aborts the launch.
        assert!(!plan.args.iter().any(|a| a == "--fsr"));
        if let Some(f_pos) = plan.args.iter().position(|a| a == "-F") {
            assert_eq!(plan.args[f_pos + 1], "fsr");
        }
        let sep_pos = plan.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(plan.args[sep_pos + 1], "myapp");
    }

    #[test]
    fn gamescope_turned_on_but_absent_launches_the_game_itself() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let host = Host {
            gamescope: None,
            session: crate::hardware::Session::Wayland,
        };
        let plan = LaunchPlan::build_on(&host, "myapp", &[], "myapp", &video, None);
        assert_eq!(plan.program, "myapp");
        assert!(plan.args.is_empty());
    }

    #[test]
    fn no_graphical_session_launches_the_game_itself() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let host = Host {
            session: crate::hardware::Session::Tty,
            ..desktop()
        };
        let plan = LaunchPlan::build_on(&host, "myapp", &[], "myapp", &video, None);
        assert_eq!(plan.program, "myapp");
    }

    #[test]
    fn test_launch_plan_nis_filter() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.gamescope_filter = GamescopeFilter::Nis;
        let plan = build("game", &video, None);
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
        let plan = build("game", &video, None);
        if let Some(f_pos) = plan.args.iter().position(|a| a == "-F") {
            assert_eq!(plan.args[f_pos + 1], "pixel");
        }
    }

    #[test]
    fn test_launch_plan_wine_fsr_env() {
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.wine_fsr_mode = WineFsrMode::Ultra;
        let plan = build("game", &video, None);
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
        let plan = build("game", &video, None);
        assert_eq!(plan.env.get("ENABLE_VKBASALT").unwrap(), "1");
        assert_eq!(
            plan.env.get("VKBASALT_CONFIG_FILE").unwrap(),
            &tmp.to_string_lossy().into_owned()
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn an_old_afmf_or_optiscaler_setting_sets_nothing() {
        // A saved `afmf` backend still loads but sets nothing:
        // `RADV_PERFTEST=afmf` is not an option RADV has.
        let video: VideoConfig = toml::from_str(
            "[frame_gen]\nenabled = true\nbackend = \"afmf\"\nafmf_experimental_enabled = true\n",
        )
        .unwrap();
        let plan = build("game", &video, None);
        assert!(!plan.env.contains_key("RADV_PERFTEST"));
        assert!(plan.env.is_empty());
    }

    #[test]
    fn graphics_disables_drop_wine_fsr_and_gamescope_render_size_for_the_launch_only() {
        use crate::graphics::rules::Tech;
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.base_width = 1720;
        video.upscaling.base_height = 720;
        let gs = gamescope::Config {
            render_width: 1720,
            render_height: 720,
            output_width: 3440,
            output_height: 1440,
            ..gamescope::Config::default()
        };
        let global = video.clone();
        let out = LaunchPlan::apply_graphics_disables(
            "SOTTR.exe",
            &[Tech::GamescopeUpscaling, Tech::WineFsr],
            &mut video,
            Some(&gs),
        )
        .unwrap();
        assert!(!video.upscaling.wine_fsr_enabled);
        assert_eq!((video.upscaling.base_width, out.render_width), (0, 0));
        assert_eq!(out.output_width, 3440, "the output size stays");
        assert!(
            global.upscaling.wine_fsr_enabled,
            "the caller's settings are untouched"
        );
        // Nothing to disable: nothing changes.
        let mut v2 = global.clone();
        let same = LaunchPlan::apply_graphics_disables("x", &[], &mut v2, Some(&gs)).unwrap();
        assert!(v2.upscaling.wine_fsr_enabled);
        assert_eq!((v2.upscaling.base_width, same.render_width), (1720, 1720));
    }

    #[test]
    fn test_launch_plan_resolution_from_upscaling() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.base_width = 1280;
        video.upscaling.base_height = 720;
        video.upscaling.target_width = 1920;
        video.upscaling.target_height = 1080;
        let plan = build("game", &video, None);
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
        let plan = build_with_args("steam", &args, &video, None);

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
        // when Gamescope upscaling is off; they must not override the
        // profile's.
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
    fn the_steam_client_command_is_never_wrapped() {
        // Wrapping `steam -applaunch` would put Gamescope around the client.
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let args = vec!["-applaunch".to_string(), "1808500".to_string()];
        let plan = build_with_args("steam", &args, &video, None);
        assert_eq!(plan.program, "steam");
        assert_eq!(plan.args, args);
    }
}
