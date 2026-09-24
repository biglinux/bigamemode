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

use super::config::{AiGraphicsConfig, FrameGeneration, Layer, Mode, Upscaler};
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
    /// The game's own feature, shown to work on the test machine, or
    /// measured better on this one.
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    /// Gamescope renders below the output size and upscales.
    pub gamescope_upscaling: bool,
    /// `WINE_FULLSCREEN_FSR`.
    pub wine_fsr: bool,
    /// lsfg-vk is on for this game.
    pub lsfg: bool,
    /// `MangoHud` is on.
    pub mangohud: bool,
    /// The `OptiScaler` version the profile's policy resolves to; `None` for
    /// the recommended release.
    pub optiscaler_version: Option<String>,
    /// What was measured for this game on this GPU ([`super::outcomes`]).
    pub measured: Vec<super::outcomes::Measurement>,
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
/// name, not translated) and technology. DLSS only where it runs: an RTX
/// card (`dlss_runs`); a GTX gets what an AMD card would.
fn native_choice(r: &Report, vendor: GpuVendor, dlss_runs: bool) -> Option<(&'static str, Tech)> {
    let n = &r.native;
    let dlss = n.dlss.as_ref().map(|_| ("DLSS", Tech::NativeDlss));
    let fsr = n.fsr.as_ref().map(|_| ("FSR", Tech::NativeFsr));
    let xess = n.xess.as_ref().map(|_| ("XeSS", Tech::NativeXess));
    match vendor {
        GpuVendor::Nvidia if dlss_runs => dlss.or(xess).or(fsr),
        GpuVendor::Intel => xess.or(fsr),
        // XeSS runs on AMD and pre-RTX GeForce through its DP4a path, at a
        // higher cost than FSR; FSR runs on everything.
        _ => fsr.or(xess),
    }
}

/// Build the plan for a game.
#[must_use]
pub fn plan(r: &Report, cfg: &AiGraphicsConfig, ctx: &Context) -> Plan {
    let mut p = plan_for_gpu(r, cfg, ctx);
    // Two GPUs and the game not running: the plan is for the card games are
    // expected to use, which is only confirmed once the game has it open.
    if cfg.mode != Mode::Off && r.gpus.len() > 1 && !r.gpus.iter().any(|g| g.renders_game) {
        if let Some(g) = r.gpu() {
            p.steps.push(Step::Note(Text::with(
                N_("this computer has more than one GPU: the plan is for %s, the one DXVK and VKD3D-Proton pick for Windows games; it is confirmed when the game runs"),
                [g.name.clone()],
            )));
        }
    }
    p
}

#[allow(clippy::too_many_lines)]
fn plan_for_gpu(r: &Report, cfg: &AiGraphicsConfig, ctx: &Context) -> Plan {
    if cfg.mode == Mode::Off {
        return nothing(
            Standing::NotRecommended,
            Text::plain(N_("AI Graphics is off for this game")),
            vec![],
        );
    }
    let vendor = r.gpu().map_or(GpuVendor::Other, |g| g.vendor);
    let fsr4 = r.gpu().is_some_and(super::report::GpuInfo::fsr4);
    // Only a confirmed RTX card counts: an unknown NVIDIA model is not
    // promised DLSS.
    let dlss_runs = r.gpu().and_then(super::report::GpuInfo::dlss) == Some(true);
    let native = native_choice(r, vendor, dlss_runs);
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
    if cfg.mode == Mode::Advanced
        && matches!(cfg.upscaler, Upscaler::NativeDlss | Upscaler::Dlaa)
        && !dlss_runs
    {
        let gpu = r.gpu().map_or_else(String::new, |g| g.name.clone());
        return nothing(
            Standing::NotRecommended,
            Text::plain(N_("DLSS does not run on this GPU")),
            vec![Step::Note(Text::with(
                N_("DLSS and DLAA need an NVIDIA RTX GPU; this game renders on %s. Choose FSR or XeSS instead"),
                [gpu],
            ))],
        );
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
            GpuVendor::Nvidia if dlss_runs => Output::Dlss,
            GpuVendor::Intel => Output::Xess,
            _ => Output::Fsr,
        },
    };
    // What this machine measured for OptiScaler taking over the game's
    // upscaler — better evidence than anything known in general, when the
    // benchmark tests settle it.
    let game_input = if n.xess.is_some() {
        Some("xess")
    } else if n.fsr.is_some() {
        Some("fsr")
    } else if n.dlss.is_some() {
        Some("dlss")
    } else {
        None
    };
    let learned = game_input.and_then(|i| {
        super::outcomes::learned(&ctx.measured.iter().collect::<Vec<_>>(), i)
    });
    let learned_output = learned.as_ref().filter(|l| l.better()).and_then(|l| {
        match l.output.as_str() {
            "fsr" => Some(Output::Fsr),
            "xess" => Some(Output::Xess),
            "dlss" if dlss_runs => Some(Output::Dlss),
            _ => None,
        }
    });
    // Against the game's own DLSS on an RTX card nothing measured here
    // compares: the runs were against the game's other upscaler.
    let measured_better = cfg.mode == Mode::Recommended
        && learned_output.is_some()
        && !(dlss_runs && n.dlss.is_some());
    let want_output = if measured_better {
        learned_output.unwrap_or(want_output)
    } else {
        want_output
    };
    // Recommended installs OptiScaler only where it beats the game's own
    // options: FSR 4 on RDNA 4 for a game that has no FSR 4, or where this
    // machine measured it faster. Everywhere else the game's own upscaler is
    // already the best this GPU can run.
    let optiscaler_worth_it = match (cfg.mode, vendor) {
        (Mode::Advanced, _) => {
            cfg.layer == Layer::OptiScaler
                || !matches!(cfg.upscaler, Upscaler::Auto | Upscaler::Off)
        }
        (_, GpuVendor::Amd) => fsr4 || measured_better,
        _ => measured_better,
    };
    let measured_note = learned.as_ref().map(|l| {
        let change = format!("{:+.1} %", l.fps_change_pct);
        let runs = l.runs.to_string();
        let input = match l.input.as_str() {
            "xess" => "XeSS",
            "fsr" => "FSR",
            _ => "DLSS",
        };
        if l.better() {
            Text::with(
                N_("measured on this computer: %s average frame rate over the game's own %s, %s runs each, with the 1% low no worse"),
                [change, input.to_owned(), runs],
            )
        } else if l.not_better() {
            Text::with(
                N_("measured on this computer: OptiScaler was not better than the game's own %s (%s average frame rate, %s runs each)"),
                [input.to_owned(), change, runs],
            )
        } else {
            Text::with(
                N_("measured on this computer, but not enough to decide (%s runs each)"),
                [runs],
            )
        }
    });
    if want_output == Output::Dlss && dlss_runs && n.dlss.is_some() {
        return keep_native(Text::plain(N_(
            "DLSS is the game's own feature and runs natively on this GPU",
        )));
    }
    if !optiscaler_worth_it {
        let mut p = keep_native(Text::plain(if vendor == GpuVendor::Amd {
            N_(
                "FSR 4 needs an RDNA 4 GPU under Proton, so OptiScaler would bring nothing the game does not have",
            )
        } else {
            N_("the game's own upscaler is the best this GPU runs")
        }));
        if let Some(t) = measured_note {
            p.steps.push(Step::Note(t));
        }
        return p;
    }
    let (input, input_name, standing, input_why) = if n.xess.is_some() {
        (
            Input::Xess,
            "XeSS",
            Standing::Recommended,
            N_(
                "OptiScaler takes over the game's XeSS — verified with Shadow of the Tomb Raider on BiGame-mode's test machine",
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
    } else if n.dlss.is_some() && vendor == GpuVendor::Nvidia {
        (
            Input::Dlss,
            "DLSS",
            Standing::Experimental,
            N_(
                "the game has only DLSS, which does not run on this GeForce: whether the game offers it anyway for OptiScaler to take over depends on the game and is not established",
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
    // Measured here beats "documented upstream".
    let standing = if measured_better {
        Standing::Recommended
    } else {
        standing
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
    let frame_gen = match (cfg.mode, cfg.frame_generation) {
        (Mode::Advanced, FrameGeneration::OptiScaler) if cfg.experimental => FrameGen::OptiFgFsr,
        _ => FrameGen::Off,
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
                ctx.optiscaler_version
                    .clone()
                    .unwrap_or_else(|| optiscaler::Release::recommended().version),
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
    if let Some(t) = measured_note {
        steps.insert(2, Step::Note(t));
    }
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

    fn named(vendor: GpuVendor, name: &str) -> GpuInfo {
        GpuInfo {
            name: name.into(),
            driver: "nvidia".into(),
            ..gpu(vendor, None)
        }
    }

    #[test]
    fn nvidia_rtx_with_native_dlss_installs_nothing() {
        let p = plan(
            &report(sottr(), named(GpuVendor::Nvidia, "AD104 [GeForce RTX 4070 Ti]")),
            &recommended(),
            &Context::default(),
        );
        assert_eq!(p.summary.english(), "the game's own DLSS");
        assert!(p.files.is_empty());
    }

    #[test]
    fn a_gtx_is_never_told_to_use_dlss() {
        // The lab laptop's GTX 1050 Ti with Shadow of the Tomb Raider, which
        // ships DLSS 2.3 and XeSS 1.1.
        let gtx = named(GpuVendor::Nvidia, "GP107M [GeForce GTX 1050 Ti Mobile]");
        let p = plan(&report(sottr(), gtx.clone()), &recommended(), &Context::default());
        assert_eq!(p.summary.english(), "the game's own XeSS");
        assert!(p.files.is_empty());
        assert!(
            !p.steps.iter().any(|s| s.text().english().contains("DLSS")),
            "{:#?}",
            p.steps
        );
        // A model the database does not name is not promised DLSS either.
        let unknown = named(GpuVendor::Nvidia, "10de:9999");
        let p = plan(&report(sottr(), unknown), &recommended(), &Context::default());
        assert_eq!(p.summary.english(), "the game's own XeSS");
        // Asked for by hand, DLSS is refused with the reason.
        let adv = AiGraphicsConfig {
            mode: Mode::Advanced,
            upscaler: Upscaler::NativeDlss,
            ..AiGraphicsConfig::default()
        };
        let p = plan(&report(sottr(), gtx), &adv, &Context::default());
        assert_eq!(p.standing, Standing::NotRecommended);
        assert_eq!(p.summary.english(), "DLSS does not run on this GPU");
        assert!(p.optiscaler.is_none());
    }

    #[test]
    fn with_two_gpus_the_plan_says_which_one_it_is_for_until_the_game_runs() {
        let mut r = report(
            sottr(),
            named(GpuVendor::Nvidia, "GP107M [GeForce GTX 1050 Ti Mobile]"),
        );
        r.gpus[0].renders_game = false;
        r.gpus.push(GpuInfo {
            card: "card1".into(),
            discrete: false,
            renders_game: false,
            ..named(GpuVendor::Intel, "Kaby Lake-H GT2 [HD Graphics 630]")
        });
        let note = |p: &Plan| {
            p.steps
                .iter()
                .any(|s| s.text().english().contains("more than one GPU"))
        };
        let p = plan(&r, &recommended(), &Context::default());
        assert!(note(&p), "{:#?}", p.steps);
        assert!(p.steps.iter().any(|s| s.text().english().contains("GTX 1050 Ti")));
        // Once the game has the GeForce open, it is a fact: no note.
        r.gpus[0].renders_game = true;
        assert!(!note(&plan(&r, &recommended(), &Context::default())));
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

    fn measured(setup: &str, fps: &[f64]) -> crate::graphics::outcomes::Measurement {
        crate::graphics::outcomes::Measurement {
            date: "2026-09-24".into(),
            game: "steam-1".into(),
            gpu: "x".into(),
            setup: crate::graphics::outcomes::Setup::parse(setup).unwrap(),
            resolution: None,
            optiscaler_version: Some("0.9.4".into()),
            avg_fps: fps.to_vec(),
            low_1pct: vec![],
        }
    }

    #[test]
    fn a_gain_measured_on_this_machine_makes_optiscaler_the_recommendation() {
        let gtx = named(GpuVendor::Nvidia, "GP107M [GeForce GTX 1050 Ti Mobile]");
        let faster = Context {
            measured: vec![
                measured("native:xess", &[40.0, 40.2, 40.1]),
                measured("optiscaler:xess:fsr", &[46.0, 46.1, 45.9]),
            ],
            ..Context::default()
        };
        let p = plan(&report(sottr(), gtx.clone()), &recommended(), &faster);
        assert_eq!(p.standing, Standing::Recommended);
        assert_eq!(
            p.summary.english(),
            "FSR 3.1 through OptiScaler, from the game's XeSS"
        );
        assert!(
            p.steps
                .iter()
                .any(|s| s.text().english().starts_with("measured on this computer: +14.7 %")),
            "{:#?}",
            p.steps
        );

        // Measured, and no faster: the game's own, and the plan says why.
        let same = Context {
            measured: vec![
                measured("native:xess", &[40.0, 41.0, 40.5]),
                measured("optiscaler:xess:fsr", &[40.4, 40.9, 40.2]),
            ],
            ..Context::default()
        };
        let p = plan(&report(sottr(), gtx), &recommended(), &same);
        assert_eq!(p.summary.english(), "the game's own XeSS");
        assert!(p.files.is_empty());
        assert!(
            p.steps
                .iter()
                .any(|s| s.text().english().contains("OptiScaler was not better"))
        );
    }

    #[test]
    fn a_measurement_against_xess_does_not_overrule_native_dlss_on_rtx() {
        let rtx = named(GpuVendor::Nvidia, "AD104 [GeForce RTX 4070 Ti]");
        let faster = Context {
            measured: vec![
                measured("native:xess", &[40.0, 40.2, 40.1]),
                measured("optiscaler:xess:fsr", &[46.0, 46.1, 45.9]),
            ],
            ..Context::default()
        };
        let p = plan(&report(sottr(), rtx), &recommended(), &faster);
        assert_eq!(p.summary.english(), "the game's own DLSS");
        assert!(p.files.is_empty());
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
