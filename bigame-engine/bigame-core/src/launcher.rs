//! Game launch orchestration: gamescope wrapping and env var injection, with
//! the Harmony Policy keeping technologies that do the same job from stacking.
//!
//! Merges a game's own launch settings (`crate::game_launch`) with the
//! global `VideoConfig` (Tuning) into a single `LaunchPlan` ready to
//! `spawn()`.
//!
//! Priority, highest first:
//!
//! 1. What the game's profile sets itself: its sizes, filter, sharpness,
//!    Wine FSR and vkBasalt take the place of Tuning's.
//! 2. The Turbo preset in force (Wine FSR, vkBasalt, a frame cap).
//! 3. Tuning's settings, for everything the game leaves to them.
//!
//! A profile's older `[gamescope]` table stands in for the game's own
//! Gamescope values when it has none.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::gamescope;
use crate::models::{FrameGenBackend, GamescopeFilter, UpscalingSettings};
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
    /// How to reach the games' GPU when another GPU drives the display.
    offload: Option<crate::hardware::Offload>,
    /// The main screen's size, Gamescope's output when nothing sets one.
    screen: Option<(u32, u32)>,
}

impl Host {
    fn detect() -> Self {
        let hw = crate::hardware::Hardware::detect();
        let render = crate::hardware::pick_render_gpu(&hw.gpus);
        Self {
            gamescope: crate::capabilities::Capabilities::detect().gamescope,
            offload: render.and_then(|i| crate::hardware::offload_for(&hw.gpus, i)),
            session: hw.session,
            screen: crate::screen::primary_size(),
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
    /// A frame cap for `MangoHud` to hold in a game that also shows its
    /// overlay: at spawn, a copy of the user's `MangoHud` configuration with
    /// `fps_limit` is written and the game pointed at it
    /// (`MANGOHUD_CONFIG` would replace their configuration, not add to it).
    pub mangohud_fps_limit: Option<u32>,
}

impl LaunchPlan {
    /// Build a launch plan for `executable` with its arguments, the policy
    /// evaluated against `logical_game` (the process name the game's profile
    /// is keyed on, which differs from `executable` for `steam -applaunch`),
    /// and the game's own Gamescope choice: Always or Never decide for this
    /// game; Automatic follows the global switch. The game's own launch
    /// settings (its profile's Display and Image quality,
    /// `crate::game_launch`) are read here and take the place of Tuning's;
    /// `gs_override` is a profile's older `[gamescope]` table, used only
    /// when the game has none of them.
    #[must_use]
    pub fn build_for_game(
        executable: &str,
        executable_args: &[String],
        logical_game: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
        gamescope_mode: gamescope::Mode,
    ) -> Self {
        let own = crate::game_settings::load(logical_game)
            .map(|s| s.launch)
            .unwrap_or_default();
        Self::build_on_with_mode(
            &Host::detect(),
            executable,
            executable_args,
            logical_game,
            video,
            gs_override,
            Some(gamescope_mode),
            &own,
            crate::turbo_preset::active_levers(),
        )
    }

    /// A plan on a given machine rather than this one, with no per-game
    /// Gamescope choice.
    #[cfg(test)]
    fn build_on(
        host: &Host,
        executable: &str,
        executable_args: &[String],
        logical_game: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
    ) -> Self {
        Self::build_on_with_mode(
            host,
            executable,
            executable_args,
            logical_game,
            video,
            gs_override,
            None,
            &crate::game_launch::GameLaunch::default(),
            crate::turbo_preset::Levers::default(),
        )
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn build_on_with_mode(
        host: &Host,
        executable: &str,
        executable_args: &[String],
        logical_game: &str,
        video: &VideoConfig,
        gs_override: Option<&gamescope::Config>,
        game_mode: Option<gamescope::Mode>,
        own: &crate::game_launch::GameLaunch,
        preset: crate::turbo_preset::Levers,
    ) -> Self {
        // Presentation-layer settings (Gamescope, Wine FSR, vkBasalt, frame
        // generation) are not a CPU power policy and do not depend on the power
        // profile: what the user configured is applied whatever Booster or
        // Turbo are doing. The harmony policy keeps enabled technologies from
        // conflicting.
        let mut effective_video = Self::apply_harmony_policy(logical_game, video);
        // The Turbo preset in force lays its choices over Tuning's, before
        // AI Graphics' own disables, which still win for their game.
        if let Some(on) = preset.wine_fsr {
            effective_video.upscaling.wine_fsr_enabled = on;
        }
        if let Some(on) = preset.vkbasalt {
            effective_video.upscaling.vkbasalt_enabled = on;
        }
        // The game's own values over both: set for this one game, they are
        // the most specific choice there is.
        effective_video.upscaling = own.over(
            &effective_video.upscaling,
            game_mode.unwrap_or(gamescope::Mode::Auto),
        );
        let own_gs = own.gamescope_override(&effective_video.upscaling);
        let gs_override = own_gs.as_ref().or(gs_override);
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
        // Wine FSR and Gamescope upscaling are two upscalers in series: when
        // Gamescope renders below its output for this game, Wine FSR is off
        // for this launch (a file from an older version can have both on).
        if effective_video.upscaling.wine_fsr_enabled
            && (crate::optimization::gamescope_upscales(&effective_video.upscaling)
                || gs_override.is_some_and(|g| g.render_width > 0 && g.render_height > 0))
            && game_mode != Some(gamescope::Mode::Disabled)
        {
            effective_video.upscaling.wine_fsr_enabled = false;
            tracing::info!(
                game = logical_game,
                "harmony: Wine FSR off for this launch — Gamescope already upscales"
            );
        }
        // The session may hold WINE_FULLSCREEN_FSR=1 (environment.d), and the
        // game inherits it: a Wine FSR switched off for this launch has to be
        // switched off explicitly.
        let wine_fsr_suppressed =
            video.upscaling.wine_fsr_enabled && !effective_video.upscaling.wine_fsr_enabled;
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
                mangohud_fps_limit: None,
            };
        }

        // ── Environment variables ─────────────────────────────────────────────
        let mut env = HashMap::new();
        Self::check_and_warn_conflicts(logical_game, &effective_video);

        collect_upscaling_env(upscaling, &mut env);
        if wine_fsr_suppressed {
            env.insert("WINE_FULLSCREEN_FSR".into(), "0".into());
        }
        // vkBasalt switched off by the preset while the session has it on.
        if video.upscaling.vkbasalt_enabled && !upscaling.vkbasalt_enabled {
            env.insert("ENABLE_VKBASALT".into(), "0".into());
        }
        env.extend(crate::turbo_preset::preset_env(preset));
        // On a hybrid laptop a native game renders on the GPU that drives the
        // panel unless it is offloaded: an OpenGL one through libglvnd, a
        // Vulkan one that takes the first device listed (SuperTuxKart did).
        // DXVK and VKD3D-Proton pick the discrete GPU themselves.
        // A value the user already set in their environment wins.
        if let Some(offload) = &host.offload {
            for (k, v) in offload.env() {
                if std::env::var_os(k).is_none() {
                    env.insert(k.to_owned(), v);
                }
            }
        }
        if disables.contains(&crate::graphics::rules::Tech::LsfgVk) {
            // The lsfg-vk layer's own off switch (its `disable_environment`).
            env.insert("DISABLE_LSFG".to_owned(), "1".to_owned());
        }

        // ── Decide program + args ─────────────────────────────────────────────
        // The game's Always/Never decides; its Automatic, or no choice at all,
        // follows the global "enable Gamescope" switch.
        let mode = match game_mode {
            Some(m @ (gamescope::Mode::Enabled | gamescope::Mode::Disabled)) => m,
            _ if upscaling.gamescope_enabled => gamescope::Mode::Enabled,
            _ => gamescope::Mode::Auto,
        };
        let merged = Self::merge_gamescope_config(upscaling, gs_override);
        // MangoHud chosen for this game (Profiles). It does not by itself make
        // Automatic wrap the game in Gamescope; when Gamescope runs anyway it
        // becomes Gamescope's own overlay, --mangoapp.
        let mangohud = crate::mangohud::mode_for(logical_game);
        let decision = gamescope::decide(mode, &merged, host.gamescope.as_ref(), host.session);
        tracing::info!(
            target: "gamescope",
            game = logical_game,
            wrap = decision.use_gamescope,
            reason = %decision.reason,
            "gamescope decision"
        );
        // A native game reads no DXVK or VKD3D variable, and Gamescope's
        // limiters (`-r`, `--framerate-limit`) did not hold SuperTuxKart
        // with its vsync off (570 FPS presented, MangoHud's log): the cap is
        // MangoHud's `fps_limit`, which waits inside the game's own swap.
        let native_cap = preset
            .frame_cap
            .filter(|fps| *fps > 0)
            .filter(|_| is_native(executable, executable_args));
        if decision.use_gamescope {
            let mut gs = gs_override.cloned().unwrap_or_default();
            // Gamescope's own overlay cannot limit the game; the game's own
            // MangoHud, which draws the same overlay, can.
            if mangohud != crate::mangohud::Mode::Off && native_cap.is_none() {
                gs.mangoapp = true;
            }
            // The preset's rate for the nested display too.
            let capped = match preset.frame_cap {
                Some(0) => {
                    gs.frame_limit = gamescope::FrameLimit::None;
                    true
                }
                Some(fps) => {
                    gs.frame_limit = gamescope::FrameLimit::NestedRefresh(fps);
                    true
                }
                None => false,
            };
            let gs_override = (gs_override.is_some() || gs.mangoapp || capped).then_some(&gs);
            let (program, mut args) =
                build_gamescope_argv(host, executable, executable_args, upscaling, gs_override);
            if let Some(offload) = &host.offload {
                keep_offload_in_the_game(&mut args, &mut env, offload);
            }
            keep_vkbasalt_in_the_game(&mut args, &mut env);
            let mut plan = Self {
                program,
                args,
                env,
                mangohud_fps_limit: None,
            };
            if let Some(fps) = native_cap {
                plan.limit_native(fps, mangohud);
            }
            plan
        } else {
            // On: MangoHud's Vulkan layer. Forced: its wrapper, which also
            // reaches OpenGL games.
            let mut plan = Self {
                program: executable.to_string(),
                args: executable_args.to_vec(),
                env,
                mangohud_fps_limit: None,
            };
            match mangohud {
                crate::mangohud::Mode::On => {
                    plan.env.insert("MANGOHUD".into(), "1".into());
                }
                crate::mangohud::Mode::Forced => {
                    plan.args
                        .insert(0, std::mem::replace(&mut plan.program, "mangohud".into()));
                }
                crate::mangohud::Mode::Off => {}
            }
            if let Some(fps) = native_cap {
                plan.limit_native(fps, mangohud);
            }
            plan
        }
    }

    /// Hold a native game at `fps` with `MangoHud`'s limiter: its wrapper in
    /// front of the game (inside Gamescope when there is one), hidden when
    /// the game's profile does not show `MangoHud`.
    fn limit_native(&mut self, fps: u32, mangohud: crate::mangohud::Mode) {
        let wrapped = self.program == "mangohud" || self.args.iter().any(|a| a == "mangohud");
        if !wrapped {
            if let Some(sep) = self.args.iter().position(|a| a == "--") {
                self.args.insert(sep + 1, "mangohud".into());
            } else {
                self.args
                    .insert(0, std::mem::replace(&mut self.program, "mangohud".into()));
            }
        }
        if mangohud == crate::mangohud::Mode::Off {
            self.env.insert(
                "MANGOHUD_CONFIG".into(),
                format!("no_display,fps_limit={fps}"),
            );
        } else {
            self.mangohud_fps_limit = Some(fps);
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
            // lsfg-vk without its DLL loads and generates nothing; the
            // entries stay as the user set them for when the DLL is back.
            effective.frame_gen.enabled = false;
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
        if let Some(fps) = self.mangohud_fps_limit {
            match write_capped_mangohud_config(fps) {
                Ok(path) => {
                    cmd.env("MANGOHUD_CONFIGFILE", path);
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "frame cap not applied"),
            }
        }
        in_own_process_group(&mut cmd);
        cmd.spawn()
            .with_context(|| format!("spawn '{}'", self.program))
    }
}

/// Whether BiGame-mode starts a native Linux program rather than a Windows
/// one through Wine or Proton.
fn is_native(executable: &str, args: &[String]) -> bool {
    let exe = |s: &str| s.to_ascii_lowercase().ends_with(".exe");
    !exe(executable) && !args.iter().any(|a| exe(a))
}

/// The user's `MangoHud` configuration with `fps_limit` added, written in the
/// runtime directory for one launch.
fn write_capped_mangohud_config(fps: u32) -> Result<std::path::PathBuf> {
    let theirs = std::fs::read_to_string(crate::mangohud::style_path()).unwrap_or_default();
    let kept: Vec<&str> = theirs
        .lines()
        .filter(|l| !l.trim_start().starts_with("fps_limit"))
        .collect();
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join("bigame-mode");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join("mangohud-fps-limit.conf");
    std::fs::write(&path, format!("{}\nfps_limit={fps}\n", kept.join("\n")))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
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
        // Tuning's sizes are Gamescope's settings like its filter: with its
        // switch off they are not applied (a game's own sizes come in
        // `gs_override`). Read regardless, they wrapped every game in an
        // upscaling Gamescope that Tuning called off — beside a Wine FSR it
        // had let on, two upscalers in series.
        let on = upscaling.gamescope_enabled;
        let pick = |general: u32, own: u32| if on && general > 0 { general } else { own };
        gamescope::Config {
            render_width: pick(upscaling.base_width, base.render_width),
            render_height: pick(upscaling.base_height, base.render_height),
            output_width: pick(upscaling.target_width, base.output_width),
            output_height: pick(upscaling.target_height, base.output_height),
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
                    GamescopeFilter::Integer => gamescope::Filter::Integer,
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

/// vkBasalt is a Vulkan layer switched on by the environment, and Gamescope
/// is a Vulkan program too. Left alone, Gamescope loads vkBasalt and filters
/// its own composited output ("vkBasalt info: effects = cas" in its log on
/// the reference desktop) — after its FSR, which already sharpens — and then
/// removes `ENABLE_VKBASALT` from the game's environment, so the game itself
/// never got the filter. Gamescope gets vkBasalt's off switch; the game gets
/// the switch back on, so the filter runs where it was asked for, in the
/// game.
fn keep_vkbasalt_in_the_game(args: &mut Vec<String>, env: &mut HashMap<String, String>) {
    // The plan's own variable, from the Video settings — the same settings
    // the session's environment.d file is written from.
    let wanted = env.get("ENABLE_VKBASALT").is_some_and(|v| v == "1");
    let Some(sep) = args.iter().position(|a| a == "--") else {
        return;
    };
    if !wanted {
        return;
    }
    env.insert("DISABLE_VKBASALT".into(), "1".into());
    for (i, a) in ["env", "-u", "DISABLE_VKBASALT", "ENABLE_VKBASALT=1"]
        .into_iter()
        .enumerate()
    {
        args.insert(sep + 1 + i, a.into());
    }
}

/// Give the render-offload variables to the game inside Gamescope, not to
/// Gamescope.
///
/// Gamescope composites where the display is. Given the offload variables it
/// picks the discrete GPU for itself: on the GTX 1050 Ti laptop (NVIDIA 580,
/// the HD 630 driving the panel) it then fails `vkImportSemaphoreFdKHR` and
/// segfaults within seconds, before the game shows a frame. With them only
/// on the game, `env K=V … <game>` after `--`, Gamescope composites on the
/// HD 630 and the game still renders on the GTX. Variables the user set
/// themselves are not in the plan's environment and are left alone.
fn keep_offload_in_the_game(
    args: &mut Vec<String>,
    env: &mut HashMap<String, String>,
    offload: &crate::hardware::Offload,
) {
    let Some(sep) = args.iter().position(|a| a == "--") else {
        return;
    };
    let assignments: Vec<String> = offload
        .env()
        .into_iter()
        .filter_map(|(k, _)| env.remove(k).map(|v| format!("{k}={v}")))
        .collect();
    if assignments.is_empty() {
        return;
    }
    for (i, a) in std::iter::once("env".to_owned())
        .chain(assignments)
        .enumerate()
    {
        args.insert(sep + 1 + i, a);
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
    host: &Host,
    executable: &str,
    executable_args: &[String],
    upscaling: &UpscalingSettings,
    gs_override: Option<&gamescope::Config>,
) -> (String, Vec<String>) {
    let caps = host.gamescope.clone().unwrap_or_default();
    let cfg =
        LaunchPlan::merge_gamescope_config(upscaling, gs_override).with_screen_output(host.screen);

    // Gamescope is left to composite on the GPU that drives the display. Told
    // to use the discrete GPU of a hybrid laptop (`--prefer-vk-device`), nested
    // Gamescope never showed a window on the lab laptop (GTX 1050 Ti rendering,
    // Intel HD 630 driving the panel): it could not hand its frames to the
    // compositor. The game inside still renders on the discrete GPU, through
    // the offload variables it inherits.
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
        let mode = crate::game_launch::wine_fsr_mode_word(upscaling.wine_fsr_mode);
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
            offload: None,
            screen: None,
        }
    }

    /// The lab laptop: GTX 1050 Ti with no panel, and a Gamescope that knows
    /// `--prefer-vk-device` (which must not be used there).
    fn hybrid_laptop() -> Host {
        Host {
            gamescope: Some(crate::capabilities::GamescopeCaps {
                version: None,
                flags: vec![
                    "prefer-vk-device".into(),
                    "F".into(),
                    "w".into(),
                    "h".into(),
                ],
            }),
            session: crate::hardware::Session::Wayland,
            offload: Some(crate::hardware::Offload::Nvidia),
            screen: None,
        }
    }

    #[test]
    fn a_game_started_on_a_hybrid_laptop_is_offloaded_to_the_discrete_gpu() {
        let video = VideoConfig::default();
        let plan = LaunchPlan::build_on(
            &hybrid_laptop(),
            "supertuxkart",
            &[],
            "supertuxkart",
            &video,
            None,
        );
        if std::env::var_os("__NV_PRIME_RENDER_OFFLOAD").is_none() {
            assert_eq!(
                plan.env
                    .get("__NV_PRIME_RENDER_OFFLOAD")
                    .map(String::as_str),
                Some("1")
            );
            assert_eq!(
                plan.env
                    .get("__GLX_VENDOR_LIBRARY_NAME")
                    .map(String::as_str),
                Some("nvidia")
            );
        }
        assert!(!plan.env.contains_key("DRI_PRIME"));
        // Nothing of the sort on a desktop whose dGPU drives the monitor.
        let plan = LaunchPlan::build_on(
            &desktop(),
            "supertuxkart",
            &[],
            "supertuxkart",
            &video,
            None,
        );
        assert!(!plan.env.contains_key("__NV_PRIME_RENDER_OFFLOAD"));
    }

    #[test]
    fn vkbasalt_runs_in_the_game_not_in_gamescope() {
        let mut env = HashMap::from([("ENABLE_VKBASALT".to_owned(), "1".to_owned())]);
        let mut args: Vec<String> = ["-f", "--", "game", "-arg"].map(str::to_owned).to_vec();
        keep_vkbasalt_in_the_game(&mut args, &mut env);
        assert_eq!(env.get("DISABLE_VKBASALT").map(String::as_str), Some("1"));
        assert_eq!(
            args,
            [
                "-f",
                "--",
                "env",
                "-u",
                "DISABLE_VKBASALT",
                "ENABLE_VKBASALT=1",
                "game",
                "-arg"
            ]
        );
    }

    #[test]
    fn without_vkbasalt_the_command_is_untouched() {
        let mut env = HashMap::new();
        let mut args: Vec<String> = ["-f", "--", "game"].map(str::to_owned).to_vec();
        keep_vkbasalt_in_the_game(&mut args, &mut env);
        assert!(env.is_empty());
        assert_eq!(args, ["-f", "--", "game"]);
    }

    #[test]
    fn gamescope_on_a_hybrid_laptop_composites_where_the_display_is() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let plan = LaunchPlan::build_on(&hybrid_laptop(), "game", &[], "game", &video, None);
        assert_eq!(plan.program, "gamescope");
        // Composited on the discrete GPU, nested Gamescope showed no window.
        assert!(!plan.args.iter().any(|a| a == "--prefer-vk-device"));
        // Gamescope itself does not get the offload variables: given them it
        // composited on the GTX and segfaulted. The game, after `--`, does.
        for k in [
            "__NV_PRIME_RENDER_OFFLOAD",
            "__GLX_VENDOR_LIBRARY_NAME",
            "__VK_LAYER_NV_optimus",
        ] {
            assert!(!plan.env.contains_key(k), "{k} reaches Gamescope");
        }
        if std::env::var_os("__NV_PRIME_RENDER_OFFLOAD").is_none() {
            let sep = plan.args.iter().position(|a| a == "--").unwrap();
            assert_eq!(plan.args[sep + 1], "env");
            let game = plan.args.iter().rposition(|a| a == "game").unwrap();
            assert!(plan.args[sep + 1..game].contains(&"__NV_PRIME_RENDER_OFFLOAD=1".to_owned()));
            assert!(
                plan.args[sep + 1..game].contains(&"__VK_LAYER_NV_optimus=NVIDIA_only".to_owned())
            );
        }
    }

    #[test]
    fn offload_goes_after_vkbasalt_and_before_the_game_inside_gamescope() {
        let mut env: HashMap<String, String> = [
            ("DRI_PRIME".to_owned(), "pci-0000_03_00_0".to_owned()),
            ("ENABLE_VKBASALT".to_owned(), "1".to_owned()),
        ]
        .into();
        let mut args: Vec<String> = ["-f", "--", "game"].map(str::to_owned).to_vec();
        let offload = crate::hardware::Offload::DriPrime("pci-0000_03_00_0".into());
        keep_offload_in_the_game(&mut args, &mut env, &offload);
        keep_vkbasalt_in_the_game(&mut args, &mut env);
        assert!(!env.contains_key("DRI_PRIME"));
        assert_eq!(
            args,
            [
                "-f",
                "--",
                "env",
                "-u",
                "DISABLE_VKBASALT",
                "ENABLE_VKBASALT=1",
                "env",
                "DRI_PRIME=pci-0000_03_00_0",
                "game"
            ]
        );
    }

    #[test]
    fn gamescope_shows_the_game_the_screen_size_when_nothing_sets_one() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let host = Host {
            gamescope: Some(crate::capabilities::GamescopeCaps {
                version: None,
                flags: ["w", "h", "W", "H", "f"].map(str::to_owned).to_vec(),
            }),
            screen: Some((1920, 1080)),
            ..desktop()
        };
        let plan = LaunchPlan::build_on(&host, "game", &[], "game", &video, None);
        assert_eq!(plan.program, "gamescope");
        let at = |flag: &str| {
            plan.args
                .iter()
                .position(|a| a == flag)
                .map(|i| plan.args[i + 1].as_str())
        };
        assert_eq!(
            (at("-W"), at("-H")),
            (Some("1920"), Some("1080")),
            "{:?}",
            plan.args
        );
        assert_eq!(at("-w"), None, "the game's size follows the output");
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
    fn a_game_set_to_never_is_not_wrapped_whatever_the_global_switch_says() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let never = LaunchPlan::build_on_with_mode(
            &desktop(),
            "game",
            &[],
            "game",
            &video,
            None,
            Some(gamescope::Mode::Disabled),
            &crate::game_launch::GameLaunch::default(),
            crate::turbo_preset::Levers::default(),
        );
        assert_eq!(never.program, "game");
        let auto = LaunchPlan::build_on_with_mode(
            &desktop(),
            "game",
            &[],
            "game",
            &video,
            None,
            Some(gamescope::Mode::Auto),
            &crate::game_launch::GameLaunch::default(),
            crate::turbo_preset::Levers::default(),
        );
        assert_eq!(
            auto.program, "gamescope",
            "Automatic follows the global switch"
        );
        video.upscaling.gamescope_enabled = false;
        let always = LaunchPlan::build_on_with_mode(
            &desktop(),
            "game",
            &[],
            "game",
            &video,
            None,
            Some(gamescope::Mode::Enabled),
            &crate::game_launch::GameLaunch::default(),
            crate::turbo_preset::Levers::default(),
        );
        assert_eq!(always.program, "gamescope");
    }

    #[test]
    fn gamescope_turned_on_but_absent_launches_the_game_itself() {
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let host = Host {
            gamescope: None,
            ..desktop()
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
        // A Gamescope that declares the options, so the test sees what is
        // emitted rather than nothing at all.
        let host = Host {
            gamescope: Some(crate::capabilities::GamescopeCaps {
                version: None,
                flags: vec!["S".into(), "F".into()],
            }),
            ..desktop()
        };
        let plan = LaunchPlan::build_on(&host, "game", &[], "game", &video, None);
        let s = plan
            .args
            .iter()
            .position(|a| a == "-S")
            .expect("-S emitted");
        assert_eq!(plan.args[s + 1], "integer");
        assert!(
            !plan.args.contains(&"pixel".to_owned()),
            "integer scaling is not the pixel filter"
        );
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
    fn a_turbo_preset_reaches_the_games_bigame_mode_starts() {
        use crate::turbo_preset::{Machine, Preset, levers};
        let desktop_fsr4 = Machine {
            vkbasalt: true,
            fsr4: true,
        };
        let host = Host {
            gamescope: Some(crate::capabilities::GamescopeCaps {
                version: None,
                flags: ["r", "w", "h", "W", "H", "f"]
                    .iter()
                    .map(|f| (*f).to_owned())
                    .collect(),
            }),
            session: crate::hardware::Session::Wayland,
            offload: None,
            screen: None,
        };
        let plan_with = |video: &VideoConfig, preset| {
            LaunchPlan::build_on_with_mode(
                &host,
                "game",
                &[],
                "game",
                video,
                None,
                None,
                &crate::game_launch::GameLaunch::default(),
                levers(preset, desktop_fsr4),
            )
        };
        // Locked 60: the Proton caps in the environment, Gamescope's -r too,
        // and for this native game MangoHud's limiter, hidden, inside it.
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        let plan = plan_with(&video, Preset::Locked60);
        assert_eq!(
            plan.env.get("VKD3D_FRAME_RATE").map(String::as_str),
            Some("60")
        );
        assert!(plan.env.contains_key("DXVK_CONFIG"));
        let r = plan.args.iter().position(|a| a == "-r").expect("-r");
        assert_eq!(plan.args[r + 1], "60");
        let sep = plan.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(plan.args[sep + 1], "mangohud");
        assert_eq!(
            plan.env.get("MANGOHUD_CONFIG").map(String::as_str),
            Some("no_display,fps_limit=60")
        );
        // Without Gamescope, the wrapper goes in front of the game.
        let plan = plan_with(&VideoConfig::default(), Preset::Locked60);
        assert_eq!(
            (plan.program.as_str(), plan.args[0].as_str()),
            ("mangohud", "game")
        );
        // A Windows game is capped by DXVK and VKD3D-Proton, not MangoHud.
        let plan = LaunchPlan::build_on_with_mode(
            &host,
            "game.exe",
            &[],
            "game.exe",
            &VideoConfig::default(),
            None,
            None,
            &crate::game_launch::GameLaunch::default(),
            levers(Preset::Locked60, desktop_fsr4),
        );
        assert_eq!(plan.program, "game.exe");
        assert!(!plan.env.contains_key("MANGOHUD_CONFIG"));
        // Enhanced over a Tuning with Wine FSR on: switched off explicitly,
        // vkBasalt on, FSR 4 asked for.
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        let plan = plan_with(&video, Preset::Enhanced);
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            plan.env.get("ENABLE_VKBASALT").map(String::as_str),
            Some("1")
        );
        assert_eq!(plan.env.get("FSR4_UPGRADE").map(String::as_str), Some("1"));
        // Standard adds nothing.
        let plan = plan_with(&VideoConfig::default(), Preset::Standard);
        assert!(plan.env.is_empty(), "{:?}", plan.env);
    }

    #[test]
    fn wine_fsr_and_gamescope_upscaling_never_run_together() {
        // A file from an older version can have both on: Gamescope's render
        // size wins for the launch, and Wine FSR is switched off explicitly,
        // because the game would inherit the session's WINE_FULLSCREEN_FSR=1.
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.gamescope_enabled = true;
        video.upscaling.base_width = 1280;
        video.upscaling.base_height = 720;
        let plan = build("game", &video, None);
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("0")
        );
        assert!(!plan.env.contains_key("WINE_FULLSCREEN_FSR_MODE"));

        // A game's own render size counts the same.
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        let own = gamescope::Config {
            render_width: 1280,
            render_height: 720,
            ..gamescope::Config::default()
        };
        let plan = build("game", &video, Some(&own));
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("0")
        );

        // Gamescope without a render size does not upscale: Wine FSR stays.
        let mut video = VideoConfig::default();
        video.upscaling.wine_fsr_enabled = true;
        video.upscaling.gamescope_enabled = true;
        let plan = build("game", &video, None);
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("1")
        );
    }

    fn build_own(
        video: &VideoConfig,
        own: &crate::game_launch::GameLaunch,
        mode: gamescope::Mode,
    ) -> LaunchPlan {
        // A Gamescope that declares the options, so the test sees them.
        let host = Host {
            gamescope: Some(crate::capabilities::GamescopeCaps {
                version: None,
                flags: ["F", "S", "r", "f", "fsr-sharpness"]
                    .map(str::to_owned)
                    .to_vec(),
            }),
            ..desktop()
        };
        LaunchPlan::build_on_with_mode(
            &host,
            "game.exe",
            &[],
            "game.exe",
            video,
            None,
            Some(mode),
            own,
            crate::turbo_preset::Levers::default(),
        )
    }

    fn arg_after<'a>(plan: &'a LaunchPlan, flag: &str) -> Option<&'a str> {
        let at = plan.args.iter().position(|a| a == flag)?;
        plan.args.get(at + 1).map(String::as_str)
    }

    #[test]
    fn a_games_own_launch_settings_take_the_place_of_tunings() {
        use crate::game_launch::GameLaunch;
        use gamescope::Mode;
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_enabled = true;
        video.upscaling.base_width = 1280;
        video.upscaling.base_height = 720;
        video.upscaling.target_width = 2560;
        video.upscaling.target_height = 1440;
        video.upscaling.wine_fsr_enabled = true;

        // Nothing of its own: Tuning's sizes, and Wine FSR off beside them.
        let plan = build_own(&video, &GameLaunch::default(), Mode::Auto);
        assert_eq!(arg_after(&plan, "-w"), Some("1280"));
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("0")
        );

        // Its own size, and "the game's own size", replace Tuning's.
        let own = GameLaunch {
            render: Some((1920, 1080)),
            ..GameLaunch::default()
        };
        let plan = build_own(&video, &own, Mode::Auto);
        assert_eq!(arg_after(&plan, "-w"), Some("1920"));
        assert_eq!(arg_after(&plan, "-W"), Some("2560"), "the output follows");
        let native = GameLaunch {
            render: Some((0, 0)),
            ..GameLaunch::default()
        };
        let plan = build_own(&video, &native, Mode::Auto);
        assert_eq!(plan.program, "gamescope");
        assert_eq!(arg_after(&plan, "-w"), None);
        // Nothing upscales it now: Tuning's Wine FSR reaches it again.
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("1")
        );

        // Its own Wine FSR and vkBasalt over the session's.
        let mut session = VideoConfig::default();
        session.upscaling.wine_fsr_enabled = true;
        let own = GameLaunch {
            wine_fsr: Some(false),
            vkbasalt: Some(true),
            ..GameLaunch::default()
        };
        let plan = build_own(&session, &own, Mode::Auto);
        assert_eq!(plan.program, "game.exe");
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            plan.env.get("ENABLE_VKBASALT").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn tunings_sizes_do_not_wrap_a_game_while_its_switch_is_off() {
        use crate::game_launch::GameLaunch;
        let mut video = VideoConfig::default();
        video.upscaling.base_width = 1280;
        video.upscaling.base_height = 720;
        video.upscaling.target_width = 2560;
        video.upscaling.target_height = 1440;
        video.upscaling.wine_fsr_enabled = true;
        let plan = build_own(&video, &GameLaunch::default(), gamescope::Mode::Auto);
        assert_eq!(plan.program, "game.exe");
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("1")
        );
        // A game set to Always gets them, and Wine FSR goes beside them.
        let plan = build_own(&video, &GameLaunch::default(), gamescope::Mode::Enabled);
        assert_eq!(arg_after(&plan, "-w"), Some("1280"));
        assert_eq!(
            plan.env.get("WINE_FULLSCREEN_FSR").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn a_game_set_to_always_gets_tunings_filter_and_its_own_frame_limit() {
        use crate::game_launch::GameLaunch;
        let mut video = VideoConfig::default();
        video.upscaling.gamescope_filter = GamescopeFilter::Nis;
        // Gamescope is off in Tuning; the game says Always.
        let own = GameLaunch {
            frame_limit: 60,
            ..GameLaunch::default()
        };
        let plan = build_own(&video, &own, gamescope::Mode::Enabled);
        assert_eq!(plan.program, "gamescope");
        assert_eq!(arg_after(&plan, "-F"), Some("nis"));
        assert_eq!(arg_after(&plan, "-r"), Some("60"));
        // Its own filter where Automatic runs Gamescope for its own size.
        video.upscaling.target_width = 2560;
        video.upscaling.target_height = 1440;
        let own = GameLaunch {
            render: Some((1280, 720)),
            filter: Some(GamescopeFilter::Fsr),
            ..GameLaunch::default()
        };
        let plan = build_own(&video, &own, gamescope::Mode::Auto);
        assert_eq!(plan.program, "gamescope");
        assert_eq!(arg_after(&plan, "-F"), Some("fsr"));
        assert_eq!(arg_after(&plan, "-w"), Some("1280"));
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
            mangohud_fps_limit: None,
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
            mangohud_fps_limit: None,
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
