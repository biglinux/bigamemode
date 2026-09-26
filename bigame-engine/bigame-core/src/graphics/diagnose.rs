//! "Why is AI Graphics not working?" — answered from what the analysis
//! already knows, as findings a person can act on.
//!
//! Every finding names what was checked, what was found, and what to do.
//! Nothing here reads anything the analysis did not: it is the analysis
//! read with the question "what stops this from working" in mind, so the
//! page and the support report can show the same answer.

use serde::Serialize;

use super::external;
use super::plan::Standing;
use super::report::Confidence;
use super::runtime::Status;
use super::scan::ProxyOwner;
use super::text::{N_, Text};
use super::{Analysis, backend::Backend};

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// As it should be.
    Ok,
    /// Worth knowing; nothing to do.
    Info,
    /// Limits what can be done, or needs a decision.
    Warning,
    /// Stops it from working.
    Problem,
}

/// One thing checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// How serious.
    pub level: Level,
    /// What was checked, marked for translation.
    pub check: &'static str,
    /// What was found.
    pub found: Text,
    /// What to do about it, when anything.
    pub action: Option<Text>,
}

fn f(level: Level, check: &'static str, found: Text, action: Option<Text>) -> Finding {
    Finding {
        level,
        check,
        found,
        action,
    }
}

/// The findings for `a`, worst first.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn diagnose(a: &Analysis) -> Vec<Finding> {
    let r = &a.report;
    let mut out = Vec::new();

    // ── The game ───────────────────────────────────────────────────────────
    match &r.executable {
        Some(e) => out.push(f(
            Level::Ok,
            N_("Executable"),
            Text::with(N_("%s"), [e.display().to_string()]),
            None,
        )),
        None => out.push(f(
            Level::Problem,
            N_("Executable"),
            Text::plain(N_("no Windows executable was found in the game's folder")),
            Some(Text::plain(N_(
                "a native Linux game has no DLL slots; nothing can be injected, and its own options are all there is",
            ))),
        )),
    }
    if r.machine == Some(super::pe::Machine::X86) {
        out.push(f(
            Level::Problem,
            N_("Architecture"),
            Text::plain(N_("32-bit game")),
            Some(Text::plain(N_(
                "OptiScaler exists only for 64-bit games; the game's own options are all there is",
            ))),
        ));
    }
    let api = r.api.api.map_or_else(
        || N_("unknown").to_owned(),
        |x| super::backend::api_name(x).to_owned(),
    );
    let api_level = match r.api.confidence {
        Confidence::Fact | Confidence::Detected => Level::Ok,
        Confidence::Likely => Level::Info,
        Confidence::Assumed => Level::Warning,
    };
    out.push(f(
        api_level,
        N_("Graphics API"),
        match r.api.translation {
            Some(t) => Text::with(N_("%s through %s (%s)"), [api, t.to_owned(), confidence(r.api.confidence)]),
            None => Text::with(N_("%s (%s)"), [api, confidence(r.api.confidence)]),
        },
        (api_level == Level::Warning).then(|| {
            Text::plain(N_(
                "run the game once: the API is confirmed from what it loads, and OptiScaler is configured for it",
            ))
        }),
    ));
    match r.gpu() {
        Some(g) => out.push(f(
            Level::Ok,
            N_("GPU"),
            Text::with(
                N_("%s (%s)%s"),
                [
                    super::report::display_name(&g.name),
                    g.family().label(),
                    if g.renders_game {
                        N_(" — renders the game").to_owned()
                    } else if r.gpus.len() > 1 {
                        N_(" — expected; confirmed when the game runs").to_owned()
                    } else {
                        String::new()
                    },
                ],
            ),
            None,
        )),
        None => out.push(f(
            Level::Problem,
            N_("GPU"),
            Text::plain(N_("no GPU was identified for this game")),
            None,
        )),
    }
    if let Some(p) = &r.proton {
        out.push(f(
            Level::Ok,
            N_("Proton"),
            Text::with(
                N_("%s · Windows %s · FSR 4 provider %s · HIP runtime %s"),
                [
                    p.tool
                        .clone()
                        .unwrap_or_else(|| N_("unknown build").to_owned()),
                    p.windows_version.clone().unwrap_or_else(|| "?".into()),
                    yes_no(p.fsr4_provider),
                    yes_no(p.hip_runtime),
                ],
            ),
            None,
        ));
    }

    // ── Vetoes ─────────────────────────────────────────────────────────────
    for ac in &r.anti_cheat {
        out.push(f(
            Level::Problem,
            N_("Anti-cheat"),
            Text::with(
                N_("%s (%s)"),
                [ac.name.clone(), ac.evidence.display().to_string()],
            ),
            Some(Text::plain(N_(
                "external graphics injection is disabled for this game; there is no override",
            ))),
        ));
    }
    if let Some(reason) = r.listed.as_ref().and_then(|e| e.block.as_ref()) {
        out.push(f(
            Level::Problem,
            N_("Game list"),
            Text::with(N_("injection blocked: %s"), [reason.clone()]),
            None,
        ));
    }

    // ── DLL slots ──────────────────────────────────────────────────────────
    for p in &r.proxies {
        let ours = r
            .installed
            .as_ref()
            .is_some_and(|m| m.entries.iter().any(|e| e.path == p.path));
        let level = match (&p.owner, ours) {
            (ProxyOwner::OptiScaler, true) => Level::Ok,
            (ProxyOwner::Microsoft | ProxyOwner::DlssNrOnAmd, _) => Level::Info,
            _ => Level::Warning,
        };
        out.push(f(
            level,
            N_("DLL slot"),
            Text::with(
                N_("%s belongs to %s%s"),
                [
                    p.slot.clone(),
                    p.owner.label().to_owned(),
                    if ours {
                        N_(" (placed by BiGame-mode)").to_owned()
                    } else {
                        String::new()
                    },
                ],
            ),
            match (&p.owner, ours, p.slot.as_str()) {
                (ProxyOwner::OptiScaler, false, _) => Some(Text::plain(N_(
                    "an OptiScaler BiGame-mode did not place: restore the game's files with the tool that put it there, or let BiGame-mode manage it after removing it",
                ))),
                (ProxyOwner::Microsoft | ProxyOwner::DlssNrOnAmd, _, _) => None,
                (_, false, "dxgi.dll") => Some(Text::plain(N_(
                    "OptiScaler needs dxgi.dll; remove this file, or load it through OptiScaler's own plugin loading, before AI Graphics can install",
                ))),
                _ => None,
            },
        ));
    }

    // ── Installed by BiGame-mode ───────────────────────────────────────────
    match &a.status {
        Status::NotInstalled => {
            if a.plan.optiscaler.is_some() {
                out.push(f(
                    Level::Info,
                    N_("OptiScaler"),
                    Text::plain(N_("not installed; the plan would install it")),
                    Some(Text::plain(N_("press Apply"))),
                ));
            }
        }
        Status::Configured => out.push(f(
            Level::Ok,
            N_("OptiScaler"),
            Text::plain(N_("installed and intact; it loads when the game starts")),
            None,
        )),
        Status::FilesChanged { files } => out.push(f(
            Level::Warning,
            N_("OptiScaler"),
            Text::with(
                N_("%s file(s) are not as installed (a game update, or another tool): %s"),
                [
                    files.len().to_string(),
                    files
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                ],
            ),
            Some(Text::plain(N_(
                "Repair puts missing files back; a changed binary belongs to whatever changed it",
            ))),
        )),
        Status::Starting => out.push(f(
            Level::Info,
            N_("OptiScaler"),
            Text::plain(N_("the game has just started; OptiScaler has not reported yet")),
            None,
        )),
        Status::Loaded { version } => out.push(f(
            Level::Warning,
            N_("OptiScaler"),
            Text::with(
                N_("loaded in the game (%s), but no upscaler created"),
                [version.clone().unwrap_or_else(|| "?".into())],
            ),
            Some(Text::plain(N_(
                "the game's own upscaler that OptiScaler takes over is off: choose it in the game's graphics menu (the plan names it)",
            ))),
        )),
        Status::Active {
            upscaler,
            fsr_generation,
            ..
        } => out.push(f(
            Level::Ok,
            N_("OptiScaler"),
            Text::with(
                N_("running %s%s"),
                [
                    upscaler.clone(),
                    match fsr_generation {
                        Some(3) => N_(" (FSR 3.1 proven by its log; FSR 4 is only ever shown by its overlay)").to_owned(),
                        _ => String::new(),
                    },
                ],
            ),
            None,
        )),
        Status::NotDetected => out.push(f(
            Level::Problem,
            N_("OptiScaler"),
            Text::plain(N_("installed, but the running game did not load it")),
            Some(Text::plain(N_(
                "the game may load its DLLs from another folder than the executable's, or a launcher started a different executable; the support report shows what the game mapped",
            ))),
        )),
        Status::Failed { errors } => out.push(f(
            Level::Problem,
            N_("OptiScaler"),
            Text::with(N_("reported a failure: %s"), [errors.join(" · ")]),
            Some(Text::plain(N_(
                "Restore puts the game's files back; a missing FidelityFX DLL is fixed by Repair",
            ))),
        )),
    }

    // ── Conflicts the plan found ───────────────────────────────────────────
    for problem in &a.plan.problems {
        let level = match problem.verdict {
            super::rules::Verdict::Blocked | super::rules::Verdict::Conflict => Level::Warning,
            _ => Level::Info,
        };
        out.push(f(
            level,
            N_("Combination"),
            Text::with(
                N_("%s + %s: %s"),
                [
                    problem.a.label().to_owned(),
                    problem.b.label().to_owned(),
                    problem.why.to_owned(),
                ],
            ),
            None,
        ));
    }
    if a.plan.standing == Standing::Blocked {
        out.push(f(
            Level::Problem,
            N_("Plan"),
            Text::plain(N_("blocked: nothing will be installed")),
            None,
        ));
    }

    // ── Native FSR 4 ───────────────────────────────────────────────────────
    if a.plan.backend == Backend::Native && r.native_fsr4_path() {
        let env = a.native.fsr4_upgrade_env;
        let (level, found, action) = match (a.native.fsr4_provider_loaded, env) {
            (Some(true), _) => (
                Level::Ok,
                Text::plain(N_(
                    "the running game loaded Proton's FSR 4 provider: the game's FSR path runs FSR 4",
                )),
                None,
            ),
            (Some(false), Some(false)) => (
                Level::Warning,
                Text::plain(N_(
                    "the game is running without FSR4_UPGRADE=1 in its environment, so Proton did not hand its FSR to the FSR 4 provider",
                )),
                Some(Text::plain(N_(
                    "Apply adds the launch option (Steam closed); the game must then be started again",
                ))),
            ),
            (Some(false), _) => (
                Level::Warning,
                Text::plain(N_(
                    "the game is running with the option set but without the FSR 4 provider loaded: FSR is off in its menu, or the game does not use the FidelityFX API for it",
                )),
                Some(Text::plain(N_(
                    "choose FSR (3.1 or newer) in the game's graphics menu",
                ))),
            ),
            (None, _) if a.plan.native_action.is_some() => (
                Level::Info,
                Text::plain(N_(
                    "available through Proton's provider once the launch option FSR4_UPGRADE=1 is set; confirmed when the game runs",
                )),
                None,
            ),
            (None, _) => (
                Level::Info,
                Text::plain(N_(
                    "expected through Proton's provider; confirmed when the game runs",
                )),
                None,
            ),
        };
        out.push(f(level, N_("FSR 4 (game's own)"), found, action));
    }

    // ── Neural rendering ───────────────────────────────────────────────────
    match &a.neural {
        external::Status::Unavailable { missing } => out.push(f(
            Level::Info,
            N_("Neural rendering"),
            Text::with(
                N_("unavailable: %s"),
                [missing
                    .iter()
                    .map(|m| m.what.to_owned())
                    .collect::<Vec<_>>()
                    .join(", ")],
            ),
            None,
        )),
        external::Status::NotInstalled => {}
        external::Status::Installed { found } => out.push(f(
            Level::Info,
            N_("Neural rendering"),
            Text::with(
                N_("DLSS-NR-on-AMD installed by you as %s; not verified until the game runs"),
                [found.proxy.clone().unwrap_or_else(|| "?".into())],
            ),
            None,
        )),
        external::Status::Loaded { .. } => out.push(f(
            Level::Info,
            N_("Neural rendering"),
            Text::plain(N_(
                "DLSS-NR-on-AMD is loaded in the game; its log has not said the pass ran",
            )),
            None,
        )),
        external::Status::Active { build, .. } => out.push(f(
            Level::Ok,
            N_("Neural rendering"),
            Text::with(
                N_("DLSS-NR-on-AMD active (build %s)"),
                [build.clone().unwrap_or_else(|| "?".into())],
            ),
            None,
        )),
        external::Status::Failed { errors, .. } => out.push(f(
            Level::Problem,
            N_("Neural rendering"),
            Text::with(N_("DLSS-NR-on-AMD failed: %s"), [errors.join(" · ")]),
            Some(Text::plain(N_(
                "its log names what is missing; BiGame-mode does not change its files",
            ))),
        )),
        external::Status::Blocked { anti_cheat } => out.push(f(
            Level::Info,
            N_("Neural rendering"),
            Text::with(N_("not offered: %s"), [anti_cheat.clone()]),
            None,
        )),
    }

    out.sort_by_key(|x| std::cmp::Reverse(x.level));
    out
}

fn confidence(c: Confidence) -> String {
    match c {
        Confidence::Fact => N_("confirmed"),
        Confidence::Detected => N_("detected from its files"),
        Confidence::Likely => N_("likely"),
        Confidence::Assumed => N_("assumed"),
    }
    .to_owned()
}

fn yes_no(b: bool) -> String {
    if b { N_("yes") } else { N_("no") }.to_owned()
}

/// The findings as a plain text page, for the support report and the
/// command line.
#[must_use]
pub fn render(findings: &[Finding]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for x in findings {
        let _ = writeln!(
            out,
            "[{}] {}: {}",
            match x.level {
                Level::Ok => "ok",
                Level::Info => "info",
                Level::Warning => "warn",
                Level::Problem => "PROBLEM",
            },
            x.check,
            x.found.english()
        );
        if let Some(a) = &x.action {
            let _ = writeln!(out, "      → {}", a.english());
        }
    }
    out
}
