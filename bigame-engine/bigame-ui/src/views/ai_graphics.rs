//! AI Graphics for one game: what was found, what BiGame-mode recommends,
//! and — only when the user asks — doing it, repairing it, or undoing it.
//!
//! The page is a dry run until *Apply* is pressed: it shows every step, what
//! to select in the game's own menu, and every file that would change.
//! Details that most people do not need (the API and how sure that is, DLL
//! slots, versions) are one click away, not on top.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::graphics::config::{
    AiGraphicsConfig, FrameGeneration, Layer, Mode, Upscaler, VersionPolicy,
};
use bigame_core::graphics::plan::{FrameGenPlan, NativeAction, Standing, Step};
use bigame_core::graphics::report::Confidence;
use bigame_core::graphics::runtime::Status;
use bigame_core::graphics::versions::Offer;
use bigame_core::graphics::{self, Analysis, Target, backend, diagnose, external};

use crate::i18n::{i18n, ni18n};

struct Page {
    target: Target,
    cfg: RefCell<AiGraphicsConfig>,
    analysis: RefCell<Option<Analysis>>,
    body: gtk4::Box,
    /// Where the installed version and any update offer go, filled once the
    /// offer is known (it may take a network request).
    versions: RefCell<Option<gtk4::Box>>,
    overlay: adw::ToastOverlay,
    apply: gtk4::Button,
    repair: gtk4::Button,
    remove: gtk4::Button,
    spinner: gtk4::Spinner,
    busy_label: gtk4::Label,
}

/// Plain, translatable text for a status.
#[must_use]
pub fn status_text(s: &Status) -> String {
    match s {
        Status::NotInstalled => i18n("Nothing installed"),
        Status::Configured => i18n("Installed — takes effect when the game starts"),
        Status::FilesChanged { files } => format!(
            "{} ({})",
            i18n("Files changed since they were installed — Repair can put missing ones back"),
            files.len()
        ),
        Status::Starting => i18n("Starting"),
        Status::Loaded { .. } => {
            i18n("Loaded — choose the upscaler named in the steps in the game's graphics menu")
        }
        Status::Active {
            upscaler,
            fsr_generation,
            ..
        } => format!(
            "{} ({})",
            i18n("Active"),
            upscaler_name(upscaler, *fsr_generation)
        ),
        Status::NotDetected => i18n("Installed, but the game did not load it"),
        Status::Failed { errors } => format!(
            "{}: {}",
            i18n("Failed"),
            errors.first().cloned().unwrap_or_default()
        ),
    }
}

/// `OptiScaler` backend ids, as people know them. FSR 4 is never claimed:
/// `fsr31` is "FSR 3.1" when the log proves it (`generation`), and plain
/// "FSR" otherwise — only `OptiScaler`'s own overlay can say FSR 4.
fn upscaler_name(backend: &str, generation: Option<u8>) -> String {
    match (backend, generation) {
        ("fsr31" | "fsr31_12", Some(3)) => "FSR 3.1".to_owned(),
        _ => upscaler_family(backend),
    }
}

fn upscaler_family(backend: &str) -> String {
    match backend {
        "fsr31" | "fsr31_12" => i18n("FSR"),
        "fsr21" | "fsr22" | "fsr21_12" | "fsr22_12" => i18n("FSR 2"),
        "xess" | "xess_12" => "XeSS".to_owned(),
        "dlss" => "DLSS".to_owned(),
        other => other.to_owned(),
    }
}

fn standing_text(s: Standing) -> (String, &'static str) {
    match s {
        Standing::Recommended => (i18n("Recommended"), "success"),
        Standing::Compatible => (i18n("Compatible — not yet verified in practice"), "accent"),
        Standing::Experimental => (i18n("Experimental"), "warning"),
        Standing::NotRecommended => (i18n("Not recommended"), "dim-label"),
        Standing::Blocked => (i18n("Blocked"), "error"),
    }
}

fn confidence_text(c: Confidence) -> String {
    match c {
        Confidence::Fact => i18n("confirmed"),
        Confidence::Detected => i18n("detected from its files"),
        Confidence::Likely => i18n("likely"),
        Confidence::Assumed => i18n("assumed — confidence low"),
    }
}

use crate::i18n::tr;

/// Start a sentence with a capital: core writes steps as clauses ("choose
/// `XeSS` in the game's menu"), and a row title reads as a sentence.
/// The plan's summary is a sentence, not a heading: let it wrap instead of
/// ending in an ellipsis (libadwaita ellipsizes group titles), as it did in
/// the Gamer theme's larger heading and would in Portuguese.
fn wrap_title(group: &adw::PreferencesGroup) {
    fn visit(widget: &gtk4::Widget) {
        if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
            if label.has_css_class("heading") {
                label.set_ellipsize(gtk4::pango::EllipsizeMode::None);
                label.set_wrap(true);
                label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            }
            return;
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            visit(&c);
            child = c.next_sibling();
        }
    }
    visit(group.upcast_ref::<gtk4::Widget>());
}

fn sentence(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .use_markup(false)
        .subtitle_selectable(true)
        .build()
}

fn step_row(step: &Step) -> adw::ActionRow {
    let (icon, text) = match step {
        Step::InGame(t) => ("input-gaming-symbolic", t),
        Step::Install(t) => ("folder-download-symbolic", t),
        Step::Disable(t) => ("action-unavailable-symbolic", t),
        Step::Keep(t) => ("object-select-symbolic", t),
        Step::Note(t) => ("dialog-information-symbolic", t),
    };
    let r = adw::ActionRow::builder()
        .title(sentence(&tr(text)))
        .use_markup(false)
        .build();
    r.set_title_lines(0);
    r.add_prefix(&gtk4::Image::from_icon_name(icon));
    r
}

// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn render(page: &Rc<Page>, a: &Analysis) {
    while let Some(child) = page.body.first_child() {
        page.body.remove(&child);
    }
    let r = &a.report;
    let p = &a.plan;

    // ── Current ──────────────────────────────────────────────────────
    // What the game has and runs on, in four rows a person reads in
    // order: GPU, API and translation, upscaling now, frame generation.
    let now = adw::PreferencesGroup::new();
    now.set_title(&i18n("Current"));
    if let Some(g) = r.gpu() {
        let mut sub = g.family().label();
        if let Some(u) = &g.userspace {
            let _ = write!(sub, " · {u}");
        }
        let _ = write!(
            sub,
            " · {}",
            if g.renders_game {
                i18n("renders the game")
            } else if r.gpus.len() > 1 {
                i18n("expected to render the game; confirmed when it runs")
            } else {
                i18n("the only GPU")
            }
        );
        now.add(&row(
            &bigame_core::graphics::report::display_name(&g.name),
            &sub,
        ));
    }
    now.add(&row(&i18n("Game API"), &api_line(r)));
    now.add(&row(&i18n("Upscaling"), &upscaling_now(a)));
    now.add(&row(&i18n("Frame generation"), &frame_gen_text(a)));
    page.body.append(&now);

    // ── Recommendation ───────────────────────────────────────────────
    let rec = adw::PreferencesGroup::new();
    rec.set_title(&sentence(&tr(&p.summary)));
    wrap_title(&rec);
    rec.set_description(Some(&if r.installed.is_some() && p.optiscaler.is_none() {
        i18n(
            "BiGame-mode installed OptiScaler in this game, and with the choice below it is not needed. Restore puts the game's own files back.",
        )
    } else if r.installed.is_some() {
        i18n("What BiGame-mode installed for this game. Restore puts the game's own files back.")
    } else {
        i18n("What BiGame-mode would do. Nothing changes until you press Apply.")
    }));
    let (standing, class) = standing_text(p.standing);
    let badge = gtk4::Label::new(Some(&standing));
    badge.add_css_class(class);
    badge.add_css_class("caption-heading");
    rec.set_header_suffix(Some(&crate::widgets::info::button(
        &i18n("How this is decided"),
        &i18n(
            "BiGame-mode picks the fewest components that give the best result for this game on \
             this GPU. If the game's own upscaler is already the best, nothing is installed. \
             OptiScaler is used where it adds something the game lacks — FSR 4 on RDNA 4 \
             graphics cards — or where a benchmark on this computer measured it faster, with \
             the 1% low no worse. Measurements stay on this computer. DLSS is offered only \
             on NVIDIA RTX cards. Frame generation is never switched on by itself: it raises the \
             presented frame rate, not the rendered one, and adds latency. Games with \
             anti-cheat get no injection at all.",
        ),
    )));
    let standing_row = adw::ActionRow::builder()
        .title(i18n("Standing"))
        .use_markup(false)
        .build();
    standing_row.add_suffix(&badge);
    rec.add(&standing_row);
    for s in &p.steps {
        rec.add(&step_row(s));
    }
    let files = adw::ExpanderRow::builder()
        .title(i18n("Files that will change"))
        .subtitle(if p.files.is_empty() {
            i18n("None")
        } else {
            format!("{}", p.files.len())
        })
        .build();
    for f in &p.files {
        files.add_row(&row(&f.display().to_string(), ""));
    }
    files.set_sensitive(!p.files.is_empty());
    rec.add(&files);
    for problem in &p.problems {
        rec.add(&row(
            &format!("{} + {}", i18n(problem.a.label()), i18n(problem.b.label())),
            &i18n(problem.why),
        ));
    }
    page.body.append(&rec);

    // ── OptiScaler version (installed games) ─────────────────────────
    let versions = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    versions.set_visible(false);
    page.body.append(&versions);
    *page.versions.borrow_mut() = Some(versions);

    // ── Neural rendering ─────────────────────────────────────────────
    page.body.append(&neural_group(page, a));

    // ── Choose yourself ──────────────────────────────────────────────
    page.body.append(&advanced_group(page));

    page.body.append(&found_group(r));
    page.body.append(&diagnose_group(a));

    // ── Buttons ──────────────────────────────────────────────────────
    let installed = r.installed.is_some();
    let option_set = bigame_core::graphics::fsr4_upgrade::is_enabled(page.target.app_id.as_deref());
    page.apply
        .set_visible(!installed && (p.optiscaler.is_some() || p.native_action.is_some()));
    page.apply
        .set_label(&if p.native_action.is_some() && p.optiscaler.is_none() {
            i18n("Add the launch option")
        } else {
            i18n("Apply")
        });
    page.repair.set_visible(installed);
    page.remove.set_visible(installed || option_set);
    page.remove.set_label(&if !installed && option_set {
        i18n("Remove the launch option")
    } else {
        i18n("Restore Game Graphics")
    });
}

/// `DirectX 12 · VKD3D-Proton · Vulkan on the host`, with how sure.
fn api_line(r: &bigame_core::graphics::report::Report) -> String {
    let api = r
        .api
        .api
        .map_or_else(|| i18n("Unknown"), |a| backend::api_name(a).to_owned());
    let mut s = api;
    match r.api.translation {
        Some(t) => {
            let _ = write!(s, " · {t} · {}", i18n("Vulkan on the host"));
        }
        None if r.executable.is_some() && r.runtime.as_deref() != Some("native") => {
            let _ = write!(
                s,
                " · {}",
                i18n("through DXVK or VKD3D-Proton, seen when the game runs")
            );
        }
        None => {}
    }
    let _ = write!(s, " — {}", confidence_text(r.api.confidence));
    s
}

/// What upscales the game now: `OptiScaler`'s live status when it is
/// installed, otherwise the game's own path and, on RDNA 4, whether the
/// FSR 4 provider was seen in the running game.
fn upscaling_now(a: &Analysis) -> String {
    if a.report.installed.is_some() {
        return status_text(&a.status);
    }
    let r = &a.report;
    let mut own = Vec::new();
    if r.native.dlss.is_some() {
        own.push("DLSS".to_owned());
    }
    if r.native.fsr.is_some() {
        own.push("FSR".to_owned());
    }
    if r.native.xess.is_some() {
        own.push("XeSS".to_owned());
    }
    let mut s = if own.is_empty() {
        i18n("The game ships no upscaler")
    } else {
        format!("{} {}", i18n("The game's own:"), own.join(", "))
    };
    if r.native_fsr4_path() {
        let _ = write!(
            s,
            " · {}",
            match (a.native.fsr4_provider_loaded, a.native.fsr4_upgrade_env) {
                (Some(true), _) => i18n("FSR 4 provider loaded in the running game"),
                (Some(false), Some(false)) => i18n("running without FSR4_UPGRADE=1: FSR 3.1"),
                (Some(false), _) =>
                    i18n("running without the FSR 4 provider: FSR is off in its menu"),
                (None, _) if a.plan.native_action == Some(NativeAction::Fsr4Upgrade) => {
                    i18n("FSR 4 available with the launch option FSR4_UPGRADE=1")
                }
                (None, _) => i18n("FSR 4 expected through Proton's provider"),
            }
        );
    }
    s
}

/// Which frame generation the game is left with, and what the running game
/// shows.
fn frame_gen_text(a: &Analysis) -> String {
    match a.plan.frame_generation {
        FrameGenPlan::Off => i18n("Off"),
        FrameGenPlan::Native => i18n("The game's own, as set in its menu"),
        FrameGenPlan::OptiScaler => {
            i18n("OptiScaler (experimental): more frames shown, not rendered, and more latency")
        }
        FrameGenPlan::LsfgVk => {
            i18n("lsfg-vk, from the Profiles page: more frames shown, not rendered")
        }
    }
}

/// Neural rendering: the external backend's state, what is missing, and
/// the page to get it from. Nothing here downloads or places a file.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn neural_group(page: &Rc<Page>, a: &Analysis) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Neural rendering"));
    group.set_description(Some(&i18n(
        "A neural pass over the game's own FSR output, through DLSS-NR-on-AMD — an external project BiGame-mode does not distribute, install or remove. Experimental: documented for Windows, not established under Proton.",
    )));
    let badge = gtk4::Label::new(Some(&i18n("Experimental")));
    badge.add_css_class("warning");
    badge.add_css_class("caption-heading");
    let backend_row = adw::ActionRow::builder()
        .title(i18n("Backend"))
        .subtitle("DLSS-NR-on-AMD")
        .use_markup(false)
        .build();
    backend_row.add_suffix(&badge);
    group.add(&backend_row);

    let (status, class) = match &a.neural {
        external::Status::Unavailable { .. } => {
            (i18n("Not available on this computer"), "dim-label")
        }
        external::Status::NotInstalled => (i18n("Available — not installed"), "accent"),
        external::Status::Installed { .. } => (
            i18n("Installed by you — not verified until the game runs"),
            "accent",
        ),
        external::Status::Loaded { .. } => (
            i18n("Loaded in the game — the pass has not reported yet"),
            "accent",
        ),
        external::Status::Active { .. } => (i18n("Active"), "success"),
        external::Status::Failed { .. } => (i18n("Failed"), "error"),
        external::Status::Blocked { .. } => {
            (i18n("Not offered: this game has anti-cheat"), "dim-label")
        }
    };
    let status_badge = gtk4::Label::new(Some(&status));
    status_badge.add_css_class(class);
    status_badge.add_css_class("caption-heading");
    let status_row = adw::ActionRow::builder()
        .title(i18n("Status"))
        .use_markup(false)
        .build();
    status_row.add_suffix(&status_badge);
    group.add(&status_row);

    match &a.neural {
        external::Status::Unavailable { missing } => {
            let exp = adw::ExpanderRow::builder()
                .title(i18n("Missing"))
                .subtitle(
                    missing
                        .iter()
                        .map(|m| i18n(m.what))
                        .collect::<Vec<_>>()
                        .join(" · "),
                )
                .build();
            for m in missing {
                let r = row(&i18n(m.what), &tr(&m.detail));
                r.set_subtitle_lines(0);
                exp.add_row(&r);
            }
            group.add(&exp);
        }
        external::Status::NotInstalled => {
            let r = adw::ActionRow::builder()
                .title(i18n("Get it from its official page"))
                .subtitle(i18n(
                    "Its license allows personal use and forbids redistribution, so BiGame-mode only links to it. Install it beside the game with its own setup, then detect again.",
                ))
                .use_markup(false)
                .build();
            r.set_subtitle_lines(0);
            let open = gtk4::Button::with_label(&i18n("Open official page"));
            open.set_valign(gtk4::Align::Center);
            open.connect_clicked(|b| {
                let launcher = gtk4::UriLauncher::new(external::OFFICIAL_URL);
                let win = b.root().and_downcast::<gtk4::Window>();
                launcher.launch(win.as_ref(), gio::Cancellable::NONE, |_| {});
            });
            r.add_suffix(&open);
            group.add(&r);
        }
        external::Status::Installed { found }
        | external::Status::Loaded { found }
        | external::Status::Active { found, .. }
        | external::Status::Failed { found, .. } => {
            let mut parts = Vec::new();
            if let Some(p) = &found.proxy {
                parts.push(format!(
                    "{} {}{}",
                    i18n("proxy"),
                    p,
                    found
                        .version
                        .as_ref()
                        .map(|v| format!(" {v}"))
                        .unwrap_or_default()
                ));
            }
            if found.config {
                parts.push(i18n("configuration"));
            }
            if found.weights {
                parts.push(i18n("converted weights"));
            }
            if let Some(m) = &found.model {
                parts.push(format!("{} {m}", i18n("model")));
            }
            group.add(&row(&i18n("Found beside the game"), &parts.join(" · ")));
            if let external::Status::Failed { errors, .. } = &a.neural {
                let r = row(&i18n("Its log says"), &errors.join(" · "));
                r.set_subtitle_lines(0);
                group.add(&r);
            }
            if let external::Status::Active { build: Some(b), .. } = &a.neural {
                group.add(&row(&i18n("Build"), b));
            }
        }
        external::Status::Blocked { anti_cheat } => {
            group.add(&row(&i18n("Anti-cheat"), anti_cheat));
        }
    }
    let again = adw::ActionRow::builder()
        .title(i18n("Detect again"))
        .subtitle(i18n("After installing or removing it with its own setup"))
        .activatable(true)
        .use_markup(false)
        .build();
    again.add_suffix(&gtk4::Image::from_icon_name("view-refresh-symbolic"));
    {
        let page = page.clone();
        again.connect_activated(move |_| refresh(&page));
    }
    group.add(&again);
    group
}

/// Diagnose: every check with what it found and what to do.
fn diagnose_group(a: &Analysis) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let findings = diagnose::diagnose(a);
    let problems = findings
        .iter()
        .filter(|f| f.level >= diagnose::Level::Warning)
        .count();
    let exp = adw::ExpanderRow::builder()
        .title(i18n("Diagnose"))
        .subtitle(if problems == 0 {
            i18n("Nothing stops AI Graphics from working here")
        } else {
            ni18n("%n thing to look at", "%n things to look at", problems)
        })
        .build();
    for f in &findings {
        let icon = match f.level {
            diagnose::Level::Ok => "object-select-symbolic",
            diagnose::Level::Info => "dialog-information-symbolic",
            diagnose::Level::Warning => "dialog-warning-symbolic",
            diagnose::Level::Problem => "dialog-error-symbolic",
        };
        let mut sub = tr(&f.found);
        if let Some(act) = &f.action {
            let _ = write!(
                sub,
                "
→ {}",
                tr(act)
            );
        }
        let r = row(&i18n(f.check), &sub);
        r.set_subtitle_lines(0);
        let img = gtk4::Image::from_icon_name(icon);
        if f.level == diagnose::Level::Problem {
            img.add_css_class("error");
        } else if f.level == diagnose::Level::Warning {
            img.add_css_class("warning");
        }
        r.add_prefix(&img);
        exp.add_row(&r);
    }
    group.add(&exp);
    group
}

/// "What was found": the evidence behind the plan, for whoever wants it.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn found_group(r: &bigame_core::graphics::report::Report) -> adw::PreferencesGroup {
    let found = adw::PreferencesGroup::new();
    let details = adw::ExpanderRow::builder()
        .title(i18n("Technical details"))
        .subtitle(i18n(
            "Graphics API and its evidence, GPU, the game's own upscalers, DLL slots, Proton",
        ))
        .build();
    let api = r
        .api
        .api
        .map_or_else(|| i18n("Unknown"), |a| format!("{a:?}").to_uppercase());
    let mut api_sub = format!("{api} — {}", confidence_text(r.api.confidence));
    if let Some(t) = r.api.translation {
        let _ = write!(api_sub, " · {t}");
    }
    details.add_row(&row(&i18n("Graphics API"), &api_sub));
    for e in &r.api.evidence {
        details.add_row(&row("", &tr(e)));
    }
    if let Some(g) = r.gpu() {
        let mut sub = g.name.clone();
        if let Some(u) = &g.userspace {
            let _ = write!(sub, " · {u}");
        }
        if let Some(v) = g.vram {
            // Rounded: drivers report a little under the marketed size.
            let _ = write!(sub, " · {} GB", (v + (1 << 29)) >> 30);
        }
        if g.renders_game {
            let _ = write!(sub, " · {}", i18n("renders the game"));
        } else if r.gpus.len() > 1 {
            let _ = write!(sub, " · {}", i18n("expected to render the game"));
        }
        if g.vendor == bigame_core::hardware::GpuVendor::Nvidia {
            let _ = write!(
                sub,
                " · {}",
                match g.dlss() {
                    Some(true) => i18n("runs DLSS"),
                    Some(false) => i18n("does not run DLSS"),
                    None => i18n("DLSS support not known"),
                }
            );
        }
        details.add_row(&row(&i18n("Graphics card"), &sub));
    }
    let n = &r.native;
    let mut native = Vec::new();
    if let Some(v) = &n.dlss {
        native.push(format!("DLSS {v}"));
    }
    if let Some(v) = &n.xess {
        native.push(format!("XeSS {v}"));
    }
    if let Some(v) = &n.fsr {
        native.push(format!("FSR {v}"));
    }
    if n.frame_gen() {
        native.push(i18n("frame generation"));
    }
    details.add_row(&row(
        &i18n("In the game"),
        &if native.is_empty() {
            i18n("No DLSS, FSR or XeSS")
        } else {
            native.join(" · ")
        },
    ));
    if let Some(exe) = &r.executable {
        let arch = match r.machine {
            Some(bigame_core::graphics::pe::Machine::X86) => " · 32-bit",
            Some(bigame_core::graphics::pe::Machine::X64) => " · 64-bit",
            _ => "",
        };
        details.add_row(&row(
            &i18n("Executable"),
            &format!("{}{arch}", exe.display()),
        ));
    }
    for proxy in &r.proxies {
        details.add_row(&row(
            &proxy.slot,
            &format!(
                "{}{}",
                i18n(proxy.owner.label()),
                proxy
                    .version
                    .as_ref()
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default()
            ),
        ));
    }
    for ac in &r.anti_cheat {
        details.add_row(&row(
            &i18n("Anti-cheat"),
            &format!("{} ({})", ac.name, ac.evidence.display()),
        ));
    }
    if let Some(p) = &r.proton {
        details.add_row(&row(
            "Proton",
            &format!(
                "{} · Windows {} · {}: {} · {}: {}",
                p.tool.clone().unwrap_or_else(|| i18n("unknown build")),
                p.windows_version.clone().unwrap_or_else(|| "?".into()),
                i18n("FSR 4 provider"),
                if p.fsr4_provider {
                    i18n("yes")
                } else {
                    i18n("no")
                },
                i18n("AMD HIP runtime"),
                if p.hip_runtime {
                    i18n("yes")
                } else {
                    i18n("no")
                },
            ),
        ));
    }
    if let Some(m) = &r.installed {
        details.add_row(&row(
            &i18n("Installed by BiGame-mode"),
            &format!(
                "{} {} · {}",
                m.source.component,
                m.source.version,
                ni18n("%n file", "%n files", m.entries.len())
            ),
        ));
    }
    found.add(&details);
    found
}

// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn advanced_group(page: &Rc<Page>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let exp = adw::ExpanderRow::builder()
        .title(i18n("Choose yourself"))
        .subtitle(i18n("Upscaler, frame generation and experimental options"))
        .expanded(page.cfg.borrow().mode == Mode::Advanced)
        .build();
    let cfg = page.cfg.borrow().clone();

    let ups = gtk4::StringList::new(&[
        &i18n("Best for this game"),
        &i18n("The game's own only"),
        "FSR (OptiScaler)",
        "XeSS (OptiScaler)",
    ]);
    let up_row = adw::ComboRow::builder()
        .title(i18n("Upscaler"))
        .model(&ups)
        .build();
    up_row.set_selected(match (cfg.mode, cfg.layer, cfg.upscaler) {
        (Mode::Advanced, Layer::Native, _) => 1,
        (Mode::Advanced, _, Upscaler::Fsr) => 2,
        (Mode::Advanced, _, Upscaler::Xess) => 3,
        _ => 0,
    });
    exp.add_row(&up_row);

    let fgs = gtk4::StringList::new(&[&i18n("Off"), &i18n("OptiScaler frame generation")]);
    let fg_row = adw::ComboRow::builder()
        .title(i18n("Frame generation"))
        .subtitle(i18n("More frames shown, not rendered; adds latency"))
        .model(&fgs)
        .build();
    fg_row.set_selected(u32::from(
        cfg.frame_generation == FrameGeneration::OptiScaler,
    ));
    exp.add_row(&fg_row);

    let exp_row = adw::SwitchRow::builder()
        .title(i18n("Allow experimental options"))
        .subtitle(i18n("Combinations reported to work but not established"))
        .active(cfg.experimental)
        .build();
    exp.add_row(&exp_row);

    // Which OptiScaler release: the tested one, the newest stable one, or
    // one version kept — the installed one, or the one already pinned.
    let tested = bigame_core::graphics::optiscaler::Release::recommended().version;
    let keep_version = match &cfg.version {
        VersionPolicy::Pinned(v) => v.clone(),
        _ => page
            .analysis
            .borrow()
            .as_ref()
            .and_then(|a| a.report.installed.as_ref())
            .map_or_else(|| tested.clone(), |m| m.source.version.clone()),
    };
    let versions = gtk4::StringList::new(&[
        &format!("{} ({tested})", i18n("Tested with BiGame-mode")),
        &i18n("Latest stable"),
        &format!("{} ({keep_version})", i18n("Keep one version")),
    ]);
    let version_row = adw::ComboRow::builder()
        .title(i18n("OptiScaler version"))
        .subtitle(i18n(
            "Used for the next install; an installed game is updated only when you choose",
        ))
        .model(&versions)
        .build();
    version_row.set_selected(match cfg.version {
        VersionPolicy::Recommended => 0,
        VersionPolicy::Latest => 1,
        VersionPolicy::Pinned(_) => 2,
    });
    {
        let page = page.clone();
        version_row.connect_selected_notify(move |r| {
            page.cfg.borrow_mut().version = match r.selected() {
                1 => VersionPolicy::Latest,
                2 => VersionPolicy::Pinned(keep_version.clone()),
                _ => VersionPolicy::Recommended,
            };
            save_settings(&page);
            refresh(&page);
        });
    }
    exp.add_row(&version_row);

    let update = {
        let page = page.clone();
        let (up_row, fg_row, exp_row) = (up_row.clone(), fg_row.clone(), exp_row.clone());
        move || {
            {
                let mut c = page.cfg.borrow_mut();
                let (mode, layer, upscaler) = match up_row.selected() {
                    1 => (Mode::Advanced, Layer::Native, Upscaler::Auto),
                    2 => (Mode::Advanced, Layer::OptiScaler, Upscaler::Fsr),
                    3 => (Mode::Advanced, Layer::OptiScaler, Upscaler::Xess),
                    _ => (Mode::Recommended, Layer::Auto, Upscaler::Auto),
                };
                c.mode = if fg_row.selected() == 1 {
                    Mode::Advanced
                } else {
                    mode
                };
                c.layer = layer;
                c.upscaler = upscaler;
                c.frame_generation = if fg_row.selected() == 1 {
                    FrameGeneration::OptiScaler
                } else {
                    FrameGeneration::Off
                };
                c.experimental = exp_row.is_active();
            }
            refresh(&page);
        }
    };
    let u1 = update.clone();
    up_row.connect_selected_notify(move |_| u1());
    let u2 = update.clone();
    fg_row.connect_selected_notify(move |_| u2());
    exp_row.connect_active_notify(move |_| update());
    group.add(&exp);
    group
}

fn busy(page: &Page, text: Option<&str>) {
    let on = text.is_some();
    page.spinner.set_visible(on);
    page.spinner.set_spinning(on);
    page.busy_label.set_visible(on);
    page.busy_label.set_text(text.unwrap_or_default());
    for b in [&page.apply, &page.repair, &page.remove] {
        b.set_sensitive(!on);
    }
}

fn refresh(page: &Rc<Page>) {
    let page = page.clone();
    glib::spawn_future_local(async move {
        busy(&page, Some(&i18n("Looking at the game…")));
        let target = page.target.clone();
        let cfg = page.cfg.borrow().clone();
        let analysis = gio::spawn_blocking(move || graphics::analyze(&target, &cfg)).await;
        busy(&page, None);
        let Ok(a) = analysis else {
            return;
        };
        let installed = a.report.installed.is_some();
        // Stored first: the page reads it while it is built.
        *page.analysis.borrow_mut() = Some(a.clone());
        render(&page, &a);
        if installed {
            let target = page.target.clone();
            let cfg = page.cfg.borrow().clone();
            if let Ok(Some(offer)) =
                gio::spawn_blocking(move || graphics::update_offer(&target, &cfg)).await
            {
                render_versions(&page, &offer);
            }
        }
    });
}

/// The installed `OptiScaler` version, and — never applied by itself — a
/// newer release to take or leave, or the previous version to go back to.
fn render_versions(page: &Rc<Page>, offer: &Offer) {
    let Some(slot) = page.versions.borrow().clone() else {
        return;
    };
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }
    let group = adw::PreferencesGroup::new();
    group.set_title("OptiScaler");
    let pinned = matches!(page.cfg.borrow().version, VersionPolicy::Pinned(_));
    group.add(&row(
        &format!("{} {}", i18n("Installed version"), offer.installed),
        &if pinned {
            i18n("Kept at this version: newer releases are not offered")
        } else {
            i18n("Newer releases are offered here; nothing is updated by itself")
        },
    ));
    if let Some(new) = &offer.available {
        let r = adw::ActionRow::builder()
            .title(format!("{} {}", i18n("Update available:"), new.version))
            .subtitle(i18n(
                "The current version stays one click away. Updating while a version works is your choice.",
            ))
            .use_markup(false)
            .build();
        r.set_subtitle_lines(0);
        let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        buttons.set_valign(gtk4::Align::Center);
        let update = gtk4::Button::with_label(&i18n("Update"));
        update.add_css_class("suggested-action");
        let skip = gtk4::Button::with_label(&i18n("Skip"));
        let keep = gtk4::Button::with_label(&i18n("Keep this version"));
        for b in [&keep, &skip, &update] {
            buttons.append(b);
        }
        r.add_suffix(&buttons);
        group.add(&r);
        {
            let (page, new) = (page.clone(), new.clone());
            update.connect_clicked(move |_| change_version(&page, Some(new.clone())));
        }
        {
            let (page, v) = (page.clone(), new.version.clone());
            skip.connect_clicked(move |_| {
                page.cfg.borrow_mut().skipped_update = Some(v.clone());
                save_settings(&page);
                refresh(&page);
            });
        }
        {
            let (page, v) = (page.clone(), offer.installed.clone());
            keep.connect_clicked(move |_| {
                page.cfg.borrow_mut().version = VersionPolicy::Pinned(v.clone());
                save_settings(&page);
                refresh(&page);
            });
        }
    }
    if let Some(prev) = &offer.previous {
        let r = row(
            &format!("{} {}", i18n("Before the last update:"), prev),
            &i18n("Go back if the new version does not work as well in this game"),
        );
        let back = gtk4::Button::with_label(&i18n("Go back"));
        back.set_valign(gtk4::Align::Center);
        r.add_suffix(&back);
        group.add(&r);
        let page = page.clone();
        back.connect_clicked(move |_| change_version(&page, None));
    }
    slot.append(&group);
    slot.set_visible(true);
}

/// Update to `to`, or go back to the previous version (`None`).
fn change_version(page: &Rc<Page>, to: Option<bigame_core::graphics::optiscaler::Release>) {
    let page = page.clone();
    glib::spawn_future_local(async move {
        if refuse_while_running(&page, &page.overlay) {
            return;
        }
        busy(&page, Some(&i18n("Downloading, checking and installing…")));
        let target = page.target.clone();
        let cfg = page.cfg.borrow().clone();
        let result = gio::spawn_blocking(move || {
            let plan = graphics::analyze(&target, &cfg).plan;
            match &to {
                Some(r) => graphics::update(&target, &plan, r),
                None => graphics::go_back(&target, &plan),
            }
        })
        .await;
        busy(&page, None);
        let text = match result {
            Ok(Ok(m)) => format!(
                "{} {} — {}",
                i18n("OptiScaler"),
                m.source.version,
                i18n("installed; the previous version can be restored here")
            ),
            Ok(Err(e)) => format!("{}: {e:#}", i18n("Not updated")),
            Err(_) => i18n("Not updated"),
        };
        page.overlay.add_toast(adw::Toast::new(&text));
        refresh(&page);
    });
}

/// Files cannot change while the game runs (its DLLs are loaded, and a
/// change takes effect only at the next start). Checked here, in the UI's
/// language, before core's own check would refuse in English.
fn refuse_while_running(page: &Page, overlay: &adw::ToastOverlay) -> bool {
    if graphics::is_running(&page.target) {
        overlay.add_toast(adw::Toast::new(&i18n(
            "Close the game first: its files are in use, and a change takes effect at the next start",
        )));
        return true;
    }
    false
}

fn save_settings(page: &Page) {
    let mut s = bigame_core::game_settings::load(&page.target.process).unwrap_or_default();
    s.ai_graphics = page.cfg.borrow().clone();
    if let Err(e) = bigame_core::game_settings::save(&page.target.process, &s) {
        tracing::warn!(error = %e, "could not save AI Graphics settings");
    }
}

/// Open AI Graphics for `target`. `mode` is the starting choice (from the
/// profile wizard, or the game's saved settings).
///
/// Building a widget tree and wiring its three actions is linear; splitting
/// it yields helpers with a single caller, so the length lint is allowed.
#[allow(clippy::too_many_lines)]
pub fn open(parent: &impl IsA<gtk4::Widget>, target: Target, mode: Option<Mode>) {
    tracing::info!(target: "graphics", game = %target.process, "AI Graphics page opened");
    let mut cfg = match bigame_core::game_settings::load(&target.process) {
        Ok(s) => s.ai_graphics,
        Err(e) => {
            tracing::warn!(target: "graphics", game = %target.process, error = %e,
                "the game's AI Graphics settings do not read; starting from the defaults");
            AiGraphicsConfig::default()
        }
    };
    if let Some(m) = mode {
        cfg.mode = m;
    }
    if cfg.mode == Mode::Off {
        cfg.mode = Mode::Recommended;
    }

    let dialog = adw::Dialog::builder()
        .title(i18n("AI Graphics"))
        .content_width(620)
        .content_height(720)
        .build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &i18n("AI Graphics"),
        &target.name,
    )));
    let report_btn = gtk4::Button::builder()
        .icon_name("document-save-symbolic")
        .tooltip_text(i18n("Save a support report"))
        .build();
    header.pack_end(&report_btn);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    let intro = gtk4::Label::new(Some(&i18n(
        "Improve image quality and performance using technologies such as DLSS, FSR, XeSS, \
         OptiScaler and compatible neural-rendering features.",
    )));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    content.append(&intro);
    content.append(&body);
    let clamp = adw::Clamp::builder()
        .maximum_size(640)
        .child(&content)
        .build();
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();

    let apply = gtk4::Button::with_label(&i18n("Apply"));
    apply.add_css_class("suggested-action");
    apply.add_css_class("pill");
    let repair = gtk4::Button::with_label(&i18n("Repair"));
    repair.add_css_class("pill");
    let remove = gtk4::Button::with_label(&i18n("Restore Game Graphics"));
    remove.add_css_class("destructive-action");
    remove.add_css_class("pill");
    let spinner = gtk4::Spinner::new();
    let busy_label = gtk4::Label::new(None);
    busy_label.add_css_class("dim-label");
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    actions.set_halign(gtk4::Align::Center);
    actions.set_margin_top(6);
    actions.set_margin_bottom(12);
    for w in [
        spinner.upcast_ref::<gtk4::Widget>(),
        busy_label.upcast_ref(),
        remove.upcast_ref(),
        repair.upcast_ref(),
        apply.upcast_ref(),
    ] {
        actions.append(w);
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    toolbar.add_bottom_bar(&actions);
    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(&toolbar));
    dialog.set_child(Some(&overlay));

    let page = Rc::new(Page {
        target,
        cfg: RefCell::new(cfg),
        analysis: RefCell::new(None),
        body,
        versions: RefCell::new(None),
        overlay: overlay.clone(),
        apply: apply.clone(),
        repair: repair.clone(),
        remove: remove.clone(),
        spinner,
        busy_label,
    });
    busy(&page, None);
    for b in [&page.apply, &page.repair, &page.remove] {
        b.set_visible(false);
    }

    {
        let page = page.clone();
        let overlay = overlay.clone();
        apply.connect_clicked(move |_| {
            let page = page.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                if refuse_while_running(&page, &overlay) {
                    return;
                }
                let native_only = page
                    .analysis
                    .borrow()
                    .as_ref()
                    .is_some_and(|a| a.plan.optiscaler.is_none() && a.plan.native_action.is_some());
                if native_only {
                    // The Native backend's one action: a Steam launch option,
                    // written with Steam closed and read back. No game file.
                    if bigame_core::steam::is_running() {
                        overlay.add_toast(adw::Toast::new(&i18n(
                            "Close Steam first: it keeps its configuration in memory and would discard the launch option",
                        )));
                        return;
                    }
                    busy(&page, Some(&i18n("Writing the launch option…")));
                    save_settings(&page);
                    let app = page.target.app_id.clone();
                    let result = gio::spawn_blocking(move || {
                        bigame_core::graphics::fsr4_upgrade::apply(app.as_deref(), true)
                    })
                    .await;
                    busy(&page, None);
                    let text = match result {
                        Ok(Ok(bigame_core::graphics::fsr4_upgrade::Applied::SteamLaunchOptions(o))) => {
                            format!("{}: {o}", i18n("Steam's launch options for this game now read"))
                        }
                        Ok(Ok(bigame_core::graphics::fsr4_upgrade::Applied::SteamRunning)) => {
                            i18n("Close Steam first: it would discard the launch option")
                        }
                        Ok(Ok(bigame_core::graphics::fsr4_upgrade::Applied::LaunchPlan)) => {
                            i18n("Not a Steam game: the variable goes into BiGame-mode's own launch")
                        }
                        Ok(Err(e)) => format!("{}: {e:#}", i18n("Nothing was changed")),
                        Err(_) => i18n("Nothing was changed"),
                    };
                    overlay.add_toast(adw::Toast::new(&text));
                    refresh(&page);
                    return;
                }
                busy(&page, Some(&i18n("Downloading, checking and installing…")));
                save_settings(&page);
                let target = page.target.clone();
                let cfg = page.cfg.borrow().clone();
                let result = gio::spawn_blocking(move || {
                    let a = graphics::analyze(&target, &cfg);
                    graphics::install(&target, &a.plan, &cfg.version)
                })
                .await;
                busy(&page, None);
                let text = match result {
                    Ok(Ok(done)) => {
                        use bigame_core::graphics::ingame::Applied;
                        let files = format!(
                            "{} ({})",
                            i18n("Installed; every replaced file was backed up"),
                            ni18n("%n file", "%n files", done.manifest.entries.len())
                        );
                        match done.game_setting {
                            Some(Applied::TurnedOn(input)) => format!(
                                "{files} · {}",
                                i18n("%s switched on in the game's settings")
                                    .replace("%s", input.label())
                            ),
                            Some(Applied::NotWritten(input, _)) => format!(
                                "{files} · {}",
                                i18n("choose %s in the game's graphics menu")
                                    .replace("%s", input.label())
                            ),
                            Some(Applied::AlreadyOn(_)) | None => files,
                        }
                    }
                    Ok(Err(e)) => format!("{}: {e:#}", i18n("Nothing was changed")),
                    Err(_) => i18n("Nothing was changed"),
                };
                overlay.add_toast(adw::Toast::new(&text));
                refresh(&page);
            });
        });
    }
    {
        let page = page.clone();
        let overlay = overlay.clone();
        repair.connect_clicked(move |_| {
            let page = page.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                if refuse_while_running(&page, &overlay) {
                    return;
                }
                busy(&page, Some(&i18n("Checking files…")));
                let target = page.target.clone();
                let result = gio::spawn_blocking(move || graphics::repair(&target)).await;
                busy(&page, None);
                let text = match result {
                    Ok(Ok(v)) if v.is_empty() => i18n("Every file is as it was installed"),
                    Ok(Ok(v)) => format!("{} ({})", i18n("Missing files put back"), v.len()),
                    Ok(Err(e)) => format!("{}: {e:#}", i18n("Could not repair")),
                    Err(_) => i18n("Could not repair"),
                };
                overlay.add_toast(adw::Toast::new(&text));
                refresh(&page);
            });
        });
    }
    {
        let page = page.clone();
        let overlay = overlay.clone();
        remove.connect_clicked(move |_| {
            let page = page.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                if refuse_while_running(&page, &overlay) {
                    return;
                }
                let installed = page
                    .analysis
                    .borrow()
                    .as_ref()
                    .is_some_and(|a| a.report.installed.is_some());
                if !installed {
                    if bigame_core::steam::is_running() {
                        overlay.add_toast(adw::Toast::new(&i18n(
                            "Close Steam first: it keeps its configuration in memory and would discard the change",
                        )));
                        return;
                    }
                    busy(&page, Some(&i18n("Removing the launch option…")));
                    let app = page.target.app_id.clone();
                    let result = gio::spawn_blocking(move || {
                        bigame_core::graphics::fsr4_upgrade::apply(app.as_deref(), false)
                    })
                    .await;
                    busy(&page, None);
                    let text = match result {
                        Ok(Ok(_)) => i18n("The launch option was removed; the game's own FSR runs as it did"),
                        Ok(Err(e)) => format!("{}: {e:#}", i18n("Could not remove it")),
                        Err(_) => i18n("Could not remove it"),
                    };
                    overlay.add_toast(adw::Toast::new(&text));
                    refresh(&page);
                    return;
                }
                busy(&page, Some(&i18n("Restoring the game's own files…")));
                let target = page.target.clone();
                let result = gio::spawn_blocking(move || graphics::remove(&target)).await;
                busy(&page, None);
                let text = match result {
                    Ok(Ok(outcomes)) => {
                        let kept = outcomes
                            .iter()
                            .filter(|o| {
                                matches!(
                                    o,
                                    bigame_core::graphics::transaction::FileOutcome::KeptChanged(_)
                                )
                            })
                            .count();
                        if kept == 0 {
                            i18n("The game's files are as they were before")
                        } else {
                            format!(
                                "{} ({kept})",
                                i18n(
                                    "Restored; files another program changed since were left alone"
                                )
                            )
                        }
                    }
                    Ok(Err(e)) => format!("{}: {e:#}", i18n("Could not restore")),
                    Err(_) => i18n("Could not restore"),
                };
                overlay.add_toast(adw::Toast::new(&text));
                refresh(&page);
            });
        });
    }

    {
        let page = page.clone();
        let overlay = overlay.clone();
        report_btn.connect_clicked(move |_| {
            let Some(a) = page.analysis.borrow().clone() else {
                return;
            };
            let page = page.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                busy(&page, Some(&i18n("Writing the report…")));
                let target = page.target.clone();
                let dest = glib::user_special_dir(glib::UserDirectory::Downloads)
                    .unwrap_or_else(glib::home_dir);
                let result = gio::spawn_blocking(move || {
                    bigame_core::graphics::support::write_report(&target, &a, &dest)
                })
                .await;
                busy(&page, None);
                let text = match result {
                    Ok(Ok(path)) => format!("{} {}", i18n("Report saved to"), path.display()),
                    Ok(Err(e)) => format!("{}: {e:#}", i18n("Could not write the report")),
                    Err(_) => i18n("Could not write the report"),
                };
                overlay.add_toast(adw::Toast::new(&text));
            });
        });
    }

    refresh(&page);
    dialog.present(Some(parent));
}
