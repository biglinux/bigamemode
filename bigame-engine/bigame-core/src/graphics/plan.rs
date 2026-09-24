//! What to do for one game: the smallest set of changes that gives the best
//! upscaling this game and this GPU can have — or nothing, when the game's
//! own options are already the best.
//!
//! A plan is a dry run. It lists every step, what the user selects in the
//! game's menu, every file that would change, and every technology that has
//! to be turned off for this game, and it changes nothing. Applying it is a
//! separate, explicit step ([`super::transaction`]).
//!
//! Verdicts are earned: "Recommended" only for a combination that was shown
//! to work on the reference machine or is the game's own feature; one
//! upstream documents but nobody has checked here is "Compatible"; one that
//! depends on spoofing or is reported but not established is
//! "Experimental".
//!
//! Every sentence is a [`Text`]: a translatable template and its values.

use std::path::PathBuf;

use serde::Serialize;

use super::config::{AiGraphicsConfig, Layer, Mode, Upscaler};
use super::optiscaler::{self, Api, FrameGen, Input, Output};
use super::pe::Machine;
use super::report::{Confidence, Report};
use super::rules::{self, Tech};
use super::scan::ProxyOwner;
use super::text::{N_, Text};
use crate::hardware::GpuVendor;

/// How good a plan is (no false precision — five levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Shown to work here, or the game's own feature.
    Recommended,
    /// Documented upstream; not checked here.
    Compatible,
    /// Depends on spoofing or is not established.
    Experimental,
    /// Nothing worth doing (or it would be worse).
    NotRecommended,
    /// Would put the user's account at risk.
    Blocked,
}

/// One thing the plan does or asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "text")]
pub enum Step {
    /// Something to select in the game's own menu.
    InGame(Text),
    /// Something BiGame-mode would install.
    Install(Text),
    /// Something BiGame-mode would turn off for this game.
    Disable(Text),
    /// Something left as it is, on purpose.
    Keep(Text),
    /// Information.
    Note(Text),
}

impl Step {
    /// The sentence.
    #[must_use]
    pub fn text(&self) -> &Text {
        match self {
            Self::InGame(t)
            | Self::Install(t)
            | Self::Disable(t)
            | Self::Keep(t)
            | Self::Note(t) => t,
        }
    }
}

/// A plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// How good.
    pub standing: Standing,
    /// One line: what the game will run.
    pub summary: Text,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// `OptiScaler` configuration, when the plan installs it.
    pub optiscaler: Option<optiscaler::Options>,
    /// Files that would be placed (relative to the install folder).
    pub files: Vec<PathBuf>,
    /// Technologies the launch must turn off for this game.
    pub disable: Vec<Tech>,
    /// Combinations worth a warning.
    pub problems: Vec<rules::Rule>,
}

/// What else is configured for the game, from video settings and the profile.
// Four independent facts about the launch, not a state machine in disguise.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    /// Gamescope renders below the output size and upscales.
    pub gamescope_upscaling: bool,
    /// `WINE_FULLSCREEN_FSR`.
    pub wine_fsr: bool,
    /// lsfg-vk is on for this game.
    pub lsfg: bool,
    /// `MangoHud` is on.
    pub mangohud: bool,
}

fn nothing(standing: Standing, summary: Text, steps: Vec<Step>) -> Plan {
    Plan {
        standing,
        summary,
        steps,
        optiscaler: None,
        files: Vec::new(),
        disable: Vec::new(),
        problems: Vec::new(),
    }
}

/// The game's own upscaler that is best for `vendor`: its name (a product
/// name, not translated) and technology.
fn native_choice(r: &Report, vendor: GpuVendor) -> Option<(&'static str, Tech)> {
    let n = &r.native;
    let dlss = n.dlss.as_ref().map(|_| ("DLSS", Tech::NativeDlss));
    let fsr = n.fsr.as_ref().map(|_| ("FSR", Tech::NativeFsr));
    let xess = n.xess.as_ref().map(|_| ("XeSS", Tech::NativeXess));
    match vendor {
        GpuVendor::Nvidia => dlss.or(xess).or(fsr),
        GpuVendor::Intel => xess.or(fsr),
        // XeSS runs on AMD through its DP4a path; FSR is AMD's own.
        _ => fsr.or(xess),
    }
}

/// Build the plan for a game.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn plan(r: &Report, cfg: &AiGraphicsConfig, ctx: &Context) -> Plan {
    if cfg.mode == Mode::Off {
        return nothing(
            Standing::NotRecommended,
            Text::plain(N_("AI Graphics is off for this game")),
            vec![],
        );
    }
    let vendor = r.gpu().map_or(GpuVendor::Other, |g| g.vendor);
    let fsr4 = r.gpu().is_some_and(super::report::GpuInfo::fsr4);
    let native = native_choice(r, vendor);
    let keep_native = |why: Text| -> Plan {
        match native {
            Some((name, _)) => nothing(
                Standing::Recommended,
                Text::with(N_("the game's own %s"), [name]),
                vec![
                    Step::InGame(Text::with(
                        N_("choose %s (Quality) in the game's graphics menu"),
                        [name],
                    )),
                    Step::Keep(Text::plain(N_("no files are changed"))),
                    Step::Note(why),
                ],
            ),
            None => nothing(
                Standing::NotRecommended,
                Text::plain(N_("no upscaler available")),
                vec![
                    Step::Note(Text::plain(N_("the game ships no DLSS, FSR or XeSS"))),
                    Step::Note(why),
                ],
            ),
        }
    };

    // Accounts first.
    if let Some(ac) = r.anti_cheat.first() {
        let mut p = keep_native(Text::plain(N_(
            "graphics injection is disabled for games with anti-cheat",
        )));
        p.steps.insert(
            0,
            Step::Note(Text::with(
                N_("%s protects this game (%s): external graphics injection is disabled to avoid compatibility problems or account penalties"),
                [ac.name.clone(), ac.evidence.display().to_string()],
            )),
        );
        p.summary = if let Some((name, _)) = native {
            Text::with(
                N_("the game's own %s — injection blocked by %s"),
                [name.to_owned(), ac.name.clone()],
            )
        } else {
            p.standing = Standing::Blocked;
            Text::with(N_("blocked by %s"), [ac.name.clone()])
        };
        return p;
    }
    if r.executable.is_none() || r.runtime.as_deref() == Some("native") {
        // No Windows executable: a native Linux game. OptiScaler, and the
        // DLL-slot approach it rests on, are for Windows games under Proton
        // or Wine; a native game's own options are all there is.
        let mut p = keep_native(Text::plain(N_(
            "this is a native Linux game: OptiScaler works with Windows games under Proton or Wine",
        )));
        if native.is_none() {
            p.summary = Text::plain(N_("native Linux game: nothing for AI Graphics to do"));
        }
        return p;
    }
    if r.machine == Some(Machine::X86) {
        return keep_native(Text::plain(N_(
            "OptiScaler exists only for 64-bit games and this one is 32-bit",
        )));
    }
    if cfg.mode == Mode::Advanced && cfg.layer == Layer::Native {
        return keep_native(Text::plain(N_("only the game's own options were chosen")));
    }

    // Where OptiScaler would take over, and what it would run.
    let n = &r.native;
    let want_output = match (cfg.mode, cfg.upscaler) {
        (Mode::Advanced, Upscaler::Xess) => Output::Xess,
        (Mode::Advanced, Upscaler::NativeDlss | Upscaler::Dlaa) => Output::Dlss,
        _ => match vendor {
            GpuVendor::Nvidia => Output::Dlss,
            GpuVendor::Intel => Output::Xess,
            _ => Output::Fsr,
        },
    };
    // Recommended installs OptiScaler only where it beats the game's own
    // options: FSR 4 on RDNA 4 for a game that has no FSR 4. Everywhere else
    // the game's own upscaler is already the best this GPU can run.
    let optiscaler_worth_it = match (cfg.mode, vendor) {
        (Mode::Advanced, _) => {
            cfg.layer == Layer::OptiScaler
                || !matches!(cfg.upscaler, Upscaler::Auto | Upscaler::Off)
        }
        (_, GpuVendor::Amd) => fsr4,
        _ => false,
    };
    if want_output == Output::Dlss && vendor == GpuVendor::Nvidia && n.dlss.is_some() {
        return keep_native(Text::plain(N_(
            "DLSS is the game's own feature and runs natively on this GPU",
        )));
    }
    if !optiscaler_worth_it {
        return keep_native(Text::plain(if vendor == GpuVendor::Amd {
            N_(
                "FSR 4 needs an RDNA 4 GPU under Proton, so OptiScaler would bring nothing the game does not have",
            )
        } else {
            N_("the game's own upscaler is the best this GPU runs")
        }));
    }
    let (input, input_name, standing, input_why) = if n.xess.is_some() {
        (
            Input::Xess,
            "XeSS",
            Standing::Recommended,
            N_(
                "OptiScaler takes over the game's XeSS — verified with Shadow of the Tomb Raider on this machine",
            ),
        )
    } else if n.fsr.is_some() {
        (
            Input::Fsr,
            "FSR",
            Standing::Compatible,
            N_(
                "OptiScaler takes over the game's FSR — documented upstream, not yet checked on this machine",
            ),
        )
    } else if n.dlss.is_some() {
        (
            Input::Dlss,
            "DLSS",
            Standing::Experimental,
            N_(
                "the game has only DLSS, which it hides on this GPU: OptiScaler has to report an NVIDIA GPU (spoofing), which can send the game down NVIDIA-only code paths",
            ),
        )
    } else {
        return nothing(
            Standing::NotRecommended,
            Text::plain(N_("no upscaler for OptiScaler to take over")),
            vec![Step::Note(Text::plain(N_(
                "OptiScaler replaces an upscaler the game already has; this game ships none",
            )))],
        );
    };
    let api = r.api.api.unwrap_or(Api::Dx12);
    if let Some(p) = r
        .proxies
        .iter()
        .find(|p| p.slot == "dxgi.dll" && p.owner != ProxyOwner::OptiScaler)
    {
        return nothing(
            Standing::NotRecommended,
            Text::plain(N_("the DLL slot OptiScaler needs is taken")),
            vec![Step::Note(Text::with(
                N_(
                    "dxgi.dll beside the game belongs to %s; it is not overwritten. Remove it, or load it through OptiScaler, before AI Graphics can install OptiScaler",
                ),
                [format!("{:?}", p.owner)],
            ))],
        );
    }
    let frame_gen = if cfg.optiscaler_frame_generation() {
        FrameGen::OptiFgFsr
    } else {
        FrameGen::Off
    };
    let o = optiscaler::Options {
        proxy: "dxgi.dll".to_owned(),
        api,
        input,
        output: want_output,
        frame_gen,
        nvidia: vendor == GpuVendor::Nvidia,
        watermark: false,
    };
    let output_name = match want_output {
        Output::Fsr if fsr4 => "FSR 4",
        Output::Fsr => "FSR 3.1",
        Output::Xess => "XeSS",
        Output::Dlss => "DLSS",
    };
    let exe_dir = r
        .executable
        .as_ref()
        .and_then(|e| e.parent())
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut files = vec![exe_dir.join(&o.proxy), exe_dir.join("OptiScaler.ini")];
    files.extend(
        optiscaler::release_files(&o)
            .into_iter()
            .map(|f| exe_dir.join(f)),
    );

    let mut steps = vec![
        Step::Install(Text::with(
            N_("OptiScaler %s as %s beside the game, with %s configured as the output"),
            [
                optiscaler::Release::recommended().version,
                o.proxy.clone(),
                output_name.to_owned(),
            ],
        )),
        Step::InGame(Text::with(
            N_(
                "choose %s in the game's graphics menu, at the quality you want — OptiScaler runs %s in its place",
            ),
            [input_name, output_name],
        )),
        Step::Note(Text::plain(input_why)),
        Step::Keep(Text::plain(N_(
            "every file that is replaced is backed up first, and Restore puts it back",
        ))),
    ];
    if r.api.confidence >= Confidence::Likely {
        steps.push(Step::Note(Text::plain(N_(
            "the game's graphics API is not certain yet; it is confirmed the first time the game runs",
        ))));
    }
    let mut disable = Vec::new();
    let mut active = vec![Tech::OptiScalerUpscaler];
    if ctx.gamescope_upscaling {
        disable.push(Tech::GamescopeUpscaling);
        steps.push(Step::Disable(Text::plain(N_(
            "Gamescope upscaling for this game: two upscalers in series",
        ))));
    }
    if ctx.wine_fsr {
        disable.push(Tech::WineFsr);
        steps.push(Step::Disable(Text::plain(N_(
            "Wine FSR for this game: two upscalers in series",
        ))));
    }
    let mut standing = standing;
    if frame_gen != FrameGen::Off {
        active.push(Tech::OptiScalerFrameGen);
        standing = standing.max(Standing::Experimental);
        steps.push(Step::Note(Text::plain(N_(
            "frame generation raises the presented frame rate, not the rendered one, and adds latency",
        ))));
        if ctx.lsfg {
            disable.push(Tech::LsfgVk);
            steps.push(Step::Disable(Text::plain(N_(
                "lsfg-vk for this game: two frame generators in series",
            ))));
        }
        if n.frame_gen() {
            steps.push(Step::InGame(Text::plain(N_(
                "turn the game's own frame generation off",
            ))));
        }
    } else if ctx.lsfg {
        active.push(Tech::LsfgVk);
    }
    if ctx.mangohud {
        active.push(Tech::MangoHud);
    }
    let problems = rules::problems(&active);
    Plan {
        standing,
        summary: Text::with(
            N_("%s through OptiScaler, from the game's %s"),
            [output_name, input_name],
        ),
        steps,
        optiscaler: Some(o),
        files,
        disable,
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::config::FrameGeneration;
    use crate::graphics::report::{ApiEvidence, GpuInfo, Native};
    use crate::graphics::scan::{AntiCheat, Proxy};

    fn gpu(vendor: GpuVendor, rdna: Option<u8>) -> GpuInfo {
        GpuInfo {
            card: "card1".into(),
            vendor,
            name: "x".into(),
            driver: "amdgpu".into(),
            userspace: None,
            vram: None,
            discrete: true,
            rdna,
            renders_game: true,
        }
    }

    fn report(native: Native, g: GpuInfo) -> Report {
        Report {
            game: "Game".into(),
            app_id: Some("1".into()),
            install_root: "/g".into(),
            executable: Some("Game.exe".into()),
            machine: Some(Machine::X64),
            runtime: None,
            api: ApiEvidence {
                api: Some(Api::Dx12),
                confidence: Confidence::Fact,
                evidence: vec![],
                translation: Some("VKD3D-Proton"),
            },
            native,
            proxies: vec![],
            anti_cheat: vec![],
            gpus: vec![g],
            render_gpu: Some(0),
            installed: None,
            scan_truncated: false,
        }
    }

    fn sottr() -> Native {
        Native {
            dlss: Some("2.3.2.0".into()),
            xess: Some("1.1.0.21".into()),
            ..Native::default()
        }
    }

    fn recommended() -> AiGraphicsConfig {
        AiGraphicsConfig {
            mode: Mode::Recommended,
            ..AiGraphicsConfig::default()
        }
    }

    #[test]
    fn sottr_on_rdna4_gets_fsr4_from_its_xess_with_the_files_listed() {
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Amd, Some(4))),
            &recommended(),
            &Context::default(),
        );
        assert_eq!(p.standing, Standing::Recommended);
        assert_eq!(
            p.summary.english(),
            "FSR 4 through OptiScaler, from the game's XeSS"
        );
        let o = p.optiscaler.unwrap();
        assert_eq!(
            (o.input, o.output, o.proxy.as_str()),
            (Input::Xess, Output::Fsr, "dxgi.dll")
        );
        assert!(p.files.contains(&PathBuf::from("dxgi.dll")));
        assert!(
            !p.files.iter().any(|f| f.ends_with("libxess.dll")),
            "the game's XeSS is not replaced"
        );
        assert!(
            p.steps
                .iter()
                .any(|s| matches!(s, Step::InGame(t) if t.english().contains("XeSS")))
        );
    }

    #[test]
    fn on_older_amd_the_game_keeps_its_own_upscaler_and_nothing_changes() {
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Amd, Some(3))),
            &recommended(),
            &Context::default(),
        );
        assert_eq!(p.standing, Standing::Recommended);
        assert!(p.optiscaler.is_none() && p.files.is_empty());
        assert_eq!(p.summary.english(), "the game's own XeSS");
    }

    #[test]
    fn nvidia_with_native_dlss_installs_nothing() {
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Nvidia, None)),
            &recommended(),
            &Context::default(),
        );
        assert_eq!(p.summary.english(), "the game's own DLSS");
        assert!(p.files.is_empty());
    }

    #[test]
    fn anti_cheat_blocks_injection_but_still_points_at_the_games_own_upscaler() {
        let mut r = report(sottr(), gpu(GpuVendor::Amd, Some(4)));
        r.anti_cheat.push(AntiCheat {
            name: "Easy Anti-Cheat".into(),
            evidence: "EasyAntiCheat".into(),
        });
        let p = plan(&r, &recommended(), &Context::default());
        assert!(p.optiscaler.is_none() && p.files.is_empty());
        assert!(p.summary.english().contains("blocked by Easy Anti-Cheat"));
        r.native = Native::default();
        assert_eq!(
            plan(&r, &recommended(), &Context::default()).standing,
            Standing::Blocked
        );
    }

    #[test]
    fn a_32_bit_game_keeps_native() {
        let mut r = report(sottr(), gpu(GpuVendor::Amd, Some(4)));
        r.machine = Some(Machine::X86);
        assert!(
            plan(&r, &recommended(), &Context::default())
                .optiscaler
                .is_none()
        );
    }

    #[test]
    fn dlss_only_needs_spoofing_and_is_experimental() {
        let n = Native {
            dlss: Some("2.3.2.0".into()),
            ..Native::default()
        };
        let p = plan(
            &report(n, gpu(GpuVendor::Amd, Some(4))),
            &recommended(),
            &Context::default(),
        );
        assert_eq!(p.standing, Standing::Experimental);
        assert_eq!(p.optiscaler.unwrap().input, Input::Dlss);
        assert!(p.files.iter().any(|f| f.ends_with("fakenvapi.dll")));
    }

    #[test]
    fn a_dxgi_owned_by_reshade_stops_the_plan_instead_of_overwriting_it() {
        let mut r = report(sottr(), gpu(GpuVendor::Amd, Some(4)));
        r.proxies.push(Proxy {
            slot: "dxgi.dll".into(),
            path: "dxgi.dll".into(),
            owner: ProxyOwner::ReShade,
            version: None,
        });
        let p = plan(&r, &recommended(), &Context::default());
        assert_eq!(p.standing, Standing::NotRecommended);
        assert!(p.files.is_empty());
    }

    #[test]
    fn gamescope_and_wine_fsr_are_turned_off_for_the_game_never_stacked() {
        let ctx = Context {
            gamescope_upscaling: true,
            wine_fsr: true,
            ..Context::default()
        };
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Amd, Some(4))),
            &recommended(),
            &ctx,
        );
        assert_eq!(p.disable, [Tech::GamescopeUpscaling, Tech::WineFsr]);
    }

    #[test]
    fn frame_generation_is_never_automatic_and_is_experimental_when_chosen() {
        let ctx = Context {
            lsfg: true,
            ..Context::default()
        };
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Amd, Some(4))),
            &recommended(),
            &ctx,
        );
        assert_eq!(p.optiscaler.as_ref().unwrap().frame_gen, FrameGen::Off);
        assert!(
            p.disable.is_empty(),
            "lsfg-vk alone with an upscaler is not stacking"
        );
        let adv = AiGraphicsConfig {
            mode: Mode::Advanced,
            layer: Layer::OptiScaler,
            frame_generation: FrameGeneration::OptiScaler,
            experimental: true,
            ..AiGraphicsConfig::default()
        };
        let p = plan(&report(sottr(), gpu(GpuVendor::Amd, Some(4))), &adv, &ctx);
        assert_eq!(p.standing, Standing::Experimental);
        assert!(p.disable.contains(&Tech::LsfgVk));
        // Without the experimental opt-in, it stays off.
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Amd, Some(4))),
            &AiGraphicsConfig {
                experimental: false,
                ..adv
            },
            &ctx,
        );
        assert_eq!(p.optiscaler.unwrap().frame_gen, FrameGen::Off);
    }

    #[test]
    fn a_native_linux_game_gets_a_plain_explanation_and_no_files() {
        let mut r = report(Native::default(), gpu(GpuVendor::Amd, Some(4)));
        r.executable = None;
        r.machine = None;
        let p = plan(&r, &recommended(), &Context::default());
        assert!(p.files.is_empty() && p.optiscaler.is_none());
        assert!(p.summary.english().contains("native Linux game"));
    }

    #[test]
    fn off_does_nothing() {
        let p = plan(
            &report(sottr(), gpu(GpuVendor::Amd, Some(4))),
            &AiGraphicsConfig::default(),
            &Context::default(),
        );
        assert!(p.optiscaler.is_none() && p.files.is_empty() && p.steps.is_empty());
    }
}
