//! AI Graphics for one game: what was found, what BiGame-mode recommends,
//! and — only when the user asks — doing it, repairing it, or undoing it.
//!
//! The page is a dry run until *Apply* is pressed: it shows every step, what
//! to select in the game's own menu, and every file that would change.
//! Details that most people do not need (the API and how sure that is, DLL
//! slots, versions) are one click away, not on top.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::graphics::config::{
    AiGraphicsConfig, FrameGeneration, Layer, Mode, Upscaler, VersionPolicy,
};
use bigame_core::graphics::plan::{NativeAction, Standing, Step};
use bigame_core::graphics::report::Confidence;
use bigame_core::graphics::runtime::Status;
use bigame_core::graphics::versions::Offer;
use bigame_core::graphics::{self, Analysis, ChoiceState, Target, backend, diagnose, external};
use bigame_core::optimization::Feature;
use bigame_core::overview::State;

use crate::i18n::{error_text, i18n, ni18n};
use crate::widgets::notice::{self, Kind, Notice};
use crate::widgets::status::Chip;

struct Page {
    target: Target,
    cfg: RefCell<AiGraphicsConfig>,
    analysis: RefCell<Option<Analysis>>,
    scroll: gtk4::ScrolledWindow,
    /// What the game has and what runs now: facts that do not depend on the
    /// choice, so the controls below them never move when a choice changes.
    now: gtk4::Box,
    /// The choice. Built once and never rebuilt: the control in use keeps
    /// its place and its focus while the rest of the page follows it.
    choice: Choice,
    /// What Apply would do, below the choice; rebuilt with each analysis.
    plan: gtk4::Box,
    /// Versions, neural rendering, technical details and Diagnose.
    extras: gtk4::Box,
    /// Where the installed version and any update offer go, filled once the
    /// offer is known (it may take a network request).
    versions: RefCell<Option<gtk4::Box>>,
    /// The Diagnose expander, to open from a failure.
    diagnose: RefCell<Option<adw::ExpanderRow>>,
    overlay: adw::ToastOverlay,
    apply: gtk4::Button,
    repair: gtk4::Button,
    remove: gtk4::Button,
    spinner: gtk4::Spinner,
    busy_label: gtk4::Label,
    /// Where the whole choice stands, beside the buttons.
    status: Chip,
    /// Set while the page moves a control itself.
    quiet: Cell<bool>,
    /// Counts analyses, so only the latest one is shown.
    generation: Cell<u64>,
    /// The upscaler was changed since the last apply.
    upscaler_touched: Cell<bool>,
    /// What [`render_now`] last showed.
    now_rows: RefCell<Vec<(String, String)>>,
}

/// The choice rows and their state.
struct Choice {
    group: adw::PreferencesGroup,
    upscaler: adw::ComboRow,
    upscaler_chip: Chip,
    frame_gen: adw::ComboRow,
    frame_gen_chip: Chip,
    experimental: adw::SwitchRow,
    version: adw::ComboRow,
    versions: gtk4::StringList,
    /// The version "Keep one version" keeps.
    keep_version: RefCell<String>,
    /// Said when what was applied failed, with the way to Diagnose.
    failure: Notice,
    /// Said when lsfg-vk also generates frames in this game.
    lsfg_conflict: Notice,
    /// Said when the game's upscaler `OptiScaler` takes over was turned off
    /// again in the game's menu.
    setting_off: Notice,
    /// Said when Wine FSR is also on for this Steam game.
    wine_fsr: Notice,
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

/// Put `a` on screen: the facts, the state of the choice, the plan and the
/// extras. The choice rows stay; what is above them depends only on the
/// game, so a change of choice never moves them.
fn render(page: &Rc<Page>, a: &Analysis) {
    keep_in_place(&page.scroll, page.choice.group.upcast_ref(), || {
        render_now(page, a);
        render_choice_state(page, a);
        render_plan(page, a);
        render_extras(page, a);
        render_buttons(page, a);
    });
}

/// Run `change`, then keep `anchor` where it was on screen: when content
/// above it grows or shrinks (the game started, a row wrapped), the view
/// scrolls by the same amount, after the new layout, instead of jumping.
fn keep_in_place(scroll: &gtk4::ScrolledWindow, anchor: &gtk4::Widget, change: impl FnOnce()) {
    let Some(content) = scroll.child() else {
        change();
        return;
    };
    let y = move |w: &gtk4::Widget| {
        w.compute_point(&content, &gtk4::graphene::Point::new(0.0, 0.0))
            .map(|p| f64::from(p.y()))
    };
    let before = y(anchor);
    change();
    let Some(before) = before else {
        return;
    };
    let (anchor, adj) = (anchor.clone(), scroll.vadjustment());
    // Measured two frames later: new rows can take a second layout pass to
    // settle (wrapping labels), and correcting from the first would itself
    // move the page.
    let frames = Cell::new(0u8);
    scroll.add_tick_callback(move |_, _| {
        frames.set(frames.get() + 1);
        if frames.get() < 2 {
            return glib::ControlFlow::Continue;
        }
        if let Some(after) = y(&anchor) {
            let delta = after - before;
            if delta.abs() >= 1.0 {
                adj.set_value(adj.value() + delta);
            }
        }
        glib::ControlFlow::Break
    });
}

fn clear(b: &gtk4::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// What the game has and runs on, in four rows a person reads in order:
/// the GPU, the API, what the game ships, and what runs now.
fn render_now(page: &Rc<Page>, a: &Analysis) {
    let r = &a.report;
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(g) = r.gpu() {
        let mut sub = tr(&g.family().label());
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
        rows.push((bigame_core::graphics::report::display_name(&g.name), sub));
    }
    rows.push((i18n("Game API"), api_line(r)));
    rows.push((i18n("The game ships"), shipped(r)));
    rows.push((i18n("Running now"), running_now(a)));
    // Rebuilt only when it says something else: a change of choice leaves
    // everything above the choice exactly where it is.
    if *page.now_rows.borrow() == rows {
        return;
    }
    clear(&page.now);
    let now = adw::PreferencesGroup::new();
    now.set_title(&i18n("Current state"));
    now.set_description(Some(&i18n("What the game has and what it runs now.")));
    for (title, subtitle) in &rows {
        now.add(&row(title, subtitle));
    }
    page.now.append(&now);
    *page.now_rows.borrow_mut() = rows;
}

/// The game's own upscalers and frame generation.
fn shipped(r: &bigame_core::graphics::report::Report) -> String {
    let n = &r.native;
    let mut own = Vec::new();
    if n.dlss.is_some() {
        own.push("DLSS".to_owned());
    }
    if n.fsr.is_some() {
        own.push("FSR".to_owned());
    }
    if n.xess.is_some() {
        own.push("XeSS".to_owned());
    }
    if n.frame_gen() {
        own.push(i18n("frame generation"));
    }
    if own.is_empty() {
        i18n("The game ships no upscaler")
    } else {
        own.join(" · ")
    }
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

/// What runs in the game now: `OptiScaler`'s live status when it is
/// installed, otherwise the game's own path and, on RDNA 4, whether the
/// FSR 4 provider was seen in the running game.
fn running_now(a: &Analysis) -> String {
    if a.report.installed.is_some() {
        let mut s = status_text(&a.status);
        if a.installed_frame_generation == Some(true) {
            let _ = write!(s, " · {}", i18n("OptiScaler frame generation"));
        }
        return s;
    }
    if a.report.native_fsr4_path() {
        return match (a.native.fsr4_provider_loaded, a.native.fsr4_upgrade_env) {
            (Some(true), _) => i18n("FSR 4 provider loaded in the running game"),
            (Some(false), Some(false)) => i18n("running without FSR4_UPGRADE=1: FSR 3.1"),
            (Some(false), _) => i18n("running without the FSR 4 provider: FSR is off in its menu"),
            (None, _) if a.fsr4_upgrade_set => i18n("FSR 4 expected through Proton's provider"),
            (None, _) => i18n("Nothing from BiGame-mode: the game's own graphics"),
        };
    }
    i18n("Nothing from BiGame-mode: the game's own graphics")
}

/// A choice's state as a short word on its chip and a sentence under it.
fn state_words(s: ChoiceState) -> (State, String, String) {
    match s {
        ChoiceState::NothingToApply => (
            State::Off,
            i18n("No change"),
            i18n("Nothing to install for this choice"),
        ),
        ChoiceState::Selected => (
            State::Waiting,
            i18n("Selected"),
            i18n("Not applied yet — press Apply"),
        ),
        ChoiceState::NeedsRestore => (
            State::Waiting,
            i18n("Selected"),
            i18n("Not applied yet — Restore Game Graphics puts the game's own files back"),
        ),
        ChoiceState::Configured => (
            State::Waiting,
            i18n("Configured"),
            i18n("Applied; it starts working when the game starts"),
        ),
        ChoiceState::Loaded => (
            State::Waiting,
            i18n("Loaded"),
            i18n("Loaded — choose the upscaler named in the steps in the game's graphics menu"),
        ),
        ChoiceState::Active => (
            State::Active,
            i18n("Active"),
            i18n("Working in the running game"),
        ),
        ChoiceState::Failed => (
            State::Error,
            i18n("Failed"),
            i18n("Applied, but the game shows a problem — Diagnose says why"),
        ),
        ChoiceState::Blocked => (
            State::Unsupported,
            i18n("Blocked"),
            i18n("Not offered for this game"),
        ),
    }
}

/// The chips and subtitles of the choice rows, and the version names.
fn render_choice_state(page: &Rc<Page>, a: &Analysis) {
    let c = &page.choice;
    let state = graphics::choice_state(a);
    let cfg = page.cfg.borrow().clone();
    if !a.pending_changes {
        page.upscaler_touched.set(false);
    }
    // A pending change belongs to the row that changed: when only frame
    // generation differs from what is installed, the upscaler still says
    // where the installed one stands.
    let fg_differs =
        cfg.optiscaler_frame_generation() != (a.installed_frame_generation == Some(true));
    let up_state = if state == ChoiceState::Selected && fg_differs && !page.upscaler_touched.get() {
        graphics::installed_state(a)
    } else {
        state
    };
    let (chip, word, text) = state_words(up_state);
    c.upscaler_chip.widget().set_visible(true);
    c.frame_gen_chip.widget().set_visible(true);
    c.upscaler_chip.set(chip, Some(&word));
    c.upscaler.set_subtitle(&text);

    let fg_state = match (
        cfg.optiscaler_frame_generation(),
        a.installed_frame_generation,
    ) {
        (true, Some(true)) if !a.pending_changes => Some(state),
        (true, _) | (false, Some(true)) => Some(ChoiceState::Selected),
        (false, _) => None,
    };
    if let Some(s) = fg_state {
        let (chip, word, text) = state_words(s);
        c.frame_gen_chip.set(chip, Some(&word));
        c.frame_gen.set_subtitle(&text);
    } else {
        c.frame_gen_chip.set(State::Off, Some(&i18n("Off")));
        c.frame_gen
            .set_subtitle(&i18n("More frames shown, not rendered; adds latency"));
    }

    // Named while the choice keeps OptiScaler's frame generation; a choice
    // that drops it resolves the pair when it is applied.
    c.lsfg_conflict.set_visible(
        cfg.optiscaler_frame_generation()
            && bigame_core::fg::layer_installed()
            && bigame_core::fg::read_profile_any(&page.target.process).0 > 1,
    );

    c.setting_off.set_visible(a.game_setting_off);

    c.failure.set_visible(state == ChoiceState::Failed);
    if state == ChoiceState::Failed {
        c.failure.set(
            Kind::Error,
            &i18n("What was applied is not working"),
            &status_text(&a.status),
        );
    }

    // "Keep one version" names the installed version once it is known.
    let tested = bigame_core::graphics::optiscaler::Release::recommended().version;
    let keep = match &cfg.version {
        VersionPolicy::Pinned(v) => v.clone(),
        _ => a
            .report
            .installed
            .as_ref()
            .map_or_else(|| tested.clone(), |m| m.source.version.clone()),
    };
    if *c.keep_version.borrow() != keep {
        page.quiet.set(true);
        let selected = c.version.selected();
        c.versions
            .splice(2, 1, &[&format!("{} ({keep})", i18n("Keep one version"))]);
        c.version.set_selected(selected);
        page.quiet.set(false);
        *c.keep_version.borrow_mut() = keep;
    }
}

/// What Apply would do: the plan's summary, standing, steps, files and the
/// technologies it switches off.
fn render_plan(page: &Rc<Page>, a: &Analysis) {
    clear(&page.plan);
    let r = &a.report;
    let p = &a.plan;
    let rec = adw::PreferencesGroup::new();
    rec.set_title(&sentence(&tr(&p.summary)));
    wrap_title(&rec);
    rec.set_description(Some(&if r.installed.is_some() && p.optiscaler.is_none() {
        i18n(
            "BiGame-mode installed OptiScaler in this game, and with the choice below it is not needed. Restore puts the game's own files back.",
        )
    } else if a.pending_changes {
        i18n(
            "Your choice differs from what is installed. Apply changes puts the game's own files back and installs this instead.",
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
             graphics cards — or where BiGame-mode's game list records it as faster for that \
             game, measured on its test machines. DLSS is offered only \
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
    if p.native_action == Some(NativeAction::Fsr4Upgrade) && !a.fsr4_upgrade_set {
        rec.add(&row(
            "FSR 4",
            &i18n("FSR 4 available with the launch option FSR4_UPGRADE=1"),
        ));
    }
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
    page.plan.append(&rec);
    for problem in &p.problems {
        let n = Notice::new(
            Kind::Conflict,
            &format!("{} + {}", i18n(problem.a.label()), i18n(problem.b.label())),
            &sentence(&i18n(problem.why)),
        );
        page.plan.append(n.widget());
    }
}

/// Versions, neural rendering, the evidence and Diagnose.
fn render_extras(page: &Rc<Page>, a: &Analysis) {
    clear(&page.extras);
    let versions = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    versions.set_visible(false);
    page.extras.append(&versions);
    *page.versions.borrow_mut() = Some(versions);
    page.extras.append(&neural_group(page, a));
    page.extras.append(&found_group(&a.report));
    let (group, expander) = diagnose_group(a);
    page.extras.append(&group);
    *page.diagnose.borrow_mut() = Some(expander);
}

/// Which buttons apply, and the one word that says where things stand.
fn render_buttons(page: &Rc<Page>, a: &Analysis) {
    let r = &a.report;
    let p = &a.plan;
    let installed = r.installed.is_some();
    let option_set = a.fsr4_upgrade_set;
    page.apply.set_visible(
        a.pending_changes || !installed && (p.optiscaler.is_some() || p.native_action.is_some()),
    );
    page.apply.set_label(&if a.pending_changes {
        i18n("Apply changes")
    } else if p.native_action.is_some() && p.optiscaler.is_none() {
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
    let (chip, word) = match graphics::choice_state(a) {
        ChoiceState::Selected => (State::Waiting, i18n("Ready to apply")),
        ChoiceState::NeedsRestore => (State::Waiting, i18n("Restore to apply")),
        ChoiceState::NothingToApply => (State::Off, i18n("Nothing to apply")),
        ChoiceState::Blocked => (State::Unsupported, i18n("Blocked")),
        ChoiceState::Failed => (State::Error, i18n("Applied, not working")),
        ChoiceState::Active => (State::Active, i18n("Applied and active")),
        ChoiceState::Configured | ChoiceState::Loaded => (State::Configured, i18n("Applied")),
    };
    page.status.set(chip, Some(&word));
    page.status.widget().set_visible(true);
}

/// Neural rendering: the external backend's state, what is missing, and
/// the page to get it from. Nothing here downloads or places a file.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn neural_group(page: &Rc<Page>, a: &Analysis) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Neural rendering (optional)"));
    group.set_description(Some(&i18n(
        "Not needed for the upscaling above. A neural pass over the game's own FSR output, through DLSS-NR-on-AMD — an external project BiGame-mode does not distribute, install or remove. Experimental: documented for Windows, not established under Proton.",
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
        external::Status::Unavailable { .. } => (
            i18n("Not available here — optional, nothing is wrong"),
            "dim-label",
        ),
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
                .title(i18n("Why it is not available"))
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
fn diagnose_group(a: &Analysis) -> (adw::PreferencesGroup, adw::ExpanderRow) {
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
    (group, exp)
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
    let version = |v: &str| {
        if v == bigame_core::graphics::report::PRESENT {
            i18n(bigame_core::graphics::report::PRESENT)
        } else {
            v.to_owned()
        }
    };
    let mut native = Vec::new();
    if let Some(v) = &n.dlss {
        native.push(format!("DLSS {}", version(v)));
    }
    if let Some(v) = &n.xess {
        native.push(format!("XeSS {}", version(v)));
    }
    if let Some(v) = &n.fsr {
        native.push(format!("FSR {}", version(v)));
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

/// The choice rows, built once. Their handlers are wired by
/// [`wire_choice`] once the page exists.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn build_choice(cfg: &AiGraphicsConfig) -> Choice {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Your choice"));
    group.set_description(Some(&i18n(
        "Recommended picks for this game and graphics card. The quality preset stays the one chosen in the game's own menu.",
    )));

    let ups = gtk4::StringList::new(&[
        &i18n("Recommended for this game"),
        &i18n("The game's own only"),
        "FSR (OptiScaler)",
        "XeSS (OptiScaler)",
    ]);
    let upscaler = adw::ComboRow::builder()
        .title(i18n("Upscaler"))
        .model(&ups)
        .selected(match (cfg.mode, cfg.layer, cfg.upscaler) {
            (Mode::Advanced, Layer::Native, _) => 1,
            (Mode::Advanced, _, Upscaler::Fsr) => 2,
            (Mode::Advanced, _, Upscaler::Xess) => 3,
            _ => 0,
        })
        .build();
    // Hidden until the first analysis says where things stand.
    crate::widgets::optimization::cap_subtitle(upscaler.upcast_ref(), 34);
    crate::widgets::optimization::keep_value_width(&upscaler);
    let upscaler_chip = Chip::new(State::Off);
    upscaler_chip.widget().set_visible(false);
    upscaler.add_suffix(upscaler_chip.widget());
    group.add(&upscaler);

    let fgs = gtk4::StringList::new(&[&i18n("Off"), &i18n("OptiScaler frame generation")]);
    let frame_gen = adw::ComboRow::builder()
        .title(i18n("Frame generation"))
        .model(&fgs)
        .selected(u32::from(cfg.optiscaler_frame_generation()))
        .build();
    crate::widgets::optimization::cap_subtitle(frame_gen.upcast_ref(), 34);
    crate::widgets::optimization::keep_value_width(&frame_gen);
    let frame_gen_chip = Chip::new(State::Off);
    frame_gen_chip.widget().set_visible(false);
    frame_gen.add_suffix(frame_gen_chip.widget());
    group.add(&frame_gen);

    let failure = Notice::new(Kind::Error, "", "");
    failure.set_visible(false);
    group.add(failure.widget());
    let lsfg_conflict = Notice::new(
        Kind::Conflict,
        &i18n("OptiScaler and lsfg-vk both generate frames in this game"),
        &i18n(
            "Two frame generators at once cause artefacts, added latency and unpredictable behaviour. Keep one.",
        ),
    );
    lsfg_conflict.set_visible(false);
    group.add(lsfg_conflict.widget());
    let setting_off = Notice::new(
        Kind::Warning,
        &i18n("The game's upscaler is off in its settings"),
        &i18n(
            "OptiScaler takes over the upscaler Apply switched on in the game, and it has been turned off since, in the game's menu. With it off, OptiScaler has nothing to run. Switch it back on here with the game closed, or choose it again in the game's graphics menu.",
        ),
    );
    setting_off.set_visible(false);
    group.add(setting_off.widget());
    let wine_fsr = Notice::new(
        Kind::Conflict,
        &i18n("Wine FSR is also on for this game"),
        &i18n(
            "OptiScaler upscales it, and Wine FSR would scale the image a second time whenever the game runs fullscreen below the display's resolution. BiGame-mode's own launch turns it off; the Steam client needs it in the game's launch options.",
        ),
    );
    wine_fsr.set_visible(false);
    group.add(wine_fsr.widget());

    // Progressive disclosure: what few people need.
    let advanced = adw::ExpanderRow::builder()
        .title(i18n("Advanced options"))
        .subtitle(i18n("Experimental combinations and the OptiScaler version"))
        .build();
    let experimental = adw::SwitchRow::builder()
        .title(i18n("Allow experimental options"))
        .subtitle(i18n(
            "Combinations reported to work but not established. OptiScaler's frame generation is one of them.",
        ))
        .active(cfg.experimental)
        .build();
    advanced.add_row(&experimental);
    let tested = bigame_core::graphics::optiscaler::Release::recommended().version;
    let keep = match &cfg.version {
        VersionPolicy::Pinned(v) => v.clone(),
        _ => tested.clone(),
    };
    let versions = gtk4::StringList::new(&[
        &format!("{} ({tested})", i18n("Tested with BiGame-mode")),
        &i18n("Latest stable"),
        &format!("{} ({keep})", i18n("Keep one version")),
    ]);
    let version = adw::ComboRow::builder()
        .title(i18n("OptiScaler version"))
        .subtitle(i18n(
            "Used for the next install; an installed game is updated only when you choose",
        ))
        .model(&versions)
        .selected(match cfg.version {
            VersionPolicy::Recommended => 0,
            VersionPolicy::Latest => 1,
            VersionPolicy::Pinned(_) => 2,
        })
        .build();
    advanced.add_row(&version);
    group.add(&advanced);
    Choice {
        group,
        upscaler,
        upscaler_chip,
        frame_gen,
        frame_gen_chip,
        experimental,
        version,
        versions,
        keep_version: RefCell::new(keep),
        failure,
        lsfg_conflict,
        setting_off,
        wine_fsr,
    }
}

/// What the choice rows do. A change updates the choice and analyses
/// again; only what depends on the choice is redrawn, below the rows.
#[allow(clippy::too_many_lines)]
fn wire_choice(page: &Rc<Page>) {
    let c = &page.choice;
    {
        let page = Rc::clone(page);
        c.upscaler.connect_selected_notify(move |r| {
            if page.quiet.get() {
                return;
            }
            page.upscaler_touched.set(true);
            {
                let mut cfg = page.cfg.borrow_mut();
                let (mode, layer, upscaler) = match r.selected() {
                    1 => (Mode::Advanced, Layer::Native, Upscaler::Auto),
                    2 => (Mode::Advanced, Layer::OptiScaler, Upscaler::Fsr),
                    3 => (Mode::Advanced, Layer::OptiScaler, Upscaler::Xess),
                    _ => (Mode::Recommended, Layer::Auto, Upscaler::Auto),
                };
                cfg.mode = if cfg.frame_generation == FrameGeneration::OptiScaler {
                    Mode::Advanced
                } else {
                    mode
                };
                cfg.layer = layer;
                cfg.upscaler = upscaler;
            }
            refresh(&page);
        });
    }
    {
        let page = Rc::clone(page);
        c.frame_gen.connect_selected_notify(move |r| {
            if page.quiet.get() {
                return;
            }
            let wants = r.selected() == 1;
            // Two frame generators never run together: lsfg-vk on for this
            // game asks which one to keep.
            let lsfg_on = wants
                && bigame_core::fg::layer_installed()
                && bigame_core::fg::read_profile_any(&page.target.process).0 > 1;
            let conflict = lsfg_on
                .then(|| {
                    bigame_core::optimization::conflict(
                        Feature::OptiScalerFrameGen,
                        Feature::LsfgVk,
                    )
                })
                .flatten();
            if let Some(conflict) = conflict {
                let (page2, row) = (Rc::clone(&page), r.clone());
                notice::ask_conflict(r, &conflict, move |use_optiscaler| {
                    if use_optiscaler {
                        let process = page2.target.process.clone();
                        match bigame_core::fg::save_for_game(
                            &process, 1, 100, false, false, 1, true,
                        ) {
                            Ok(()) => page2.overlay.add_toast(adw::Toast::new(&i18n(
                                "lsfg-vk was turned off for this game",
                            ))),
                            Err(e) => page2.overlay.add_toast(adw::Toast::new(&format!(
                                "{}: {}",
                                i18n("Could not turn lsfg-vk off"),
                                error_text(&e)
                            ))),
                        }
                        choose_frame_generation(&page2, true);
                    } else {
                        page2.quiet.set(true);
                        row.set_selected(0);
                        page2.quiet.set(false);
                    }
                });
                return;
            }
            choose_frame_generation(&page, wants);
        });
    }
    {
        let page = Rc::clone(page);
        c.experimental.connect_active_notify(move |r| {
            if page.quiet.get() {
                return;
            }
            page.cfg.borrow_mut().experimental = r.is_active();
            if !r.is_active() && page.cfg.borrow().frame_generation == FrameGeneration::OptiScaler {
                // OptiScaler's frame generation is experimental: it goes too.
                page.quiet.set(true);
                page.choice.frame_gen.set_selected(0);
                page.quiet.set(false);
                page.cfg.borrow_mut().frame_generation = FrameGeneration::Off;
            }
            refresh(&page);
        });
    }
    {
        let page = Rc::clone(page);
        c.version.connect_selected_notify(move |r| {
            if page.quiet.get() {
                return;
            }
            let keep = page.choice.keep_version.borrow().clone();
            page.cfg.borrow_mut().version = match r.selected() {
                1 => VersionPolicy::Latest,
                2 => VersionPolicy::Pinned(keep),
                _ => VersionPolicy::Recommended,
            };
            save_settings(&page);
            refresh(&page);
        });
    }
    {
        let page = Rc::clone(page);
        c.lsfg_conflict.add_action(
            &i18n("Keep %s").replace("%s", &notice::feature_name(Feature::OptiScalerFrameGen)),
            false,
            move |_| {
                let process = page.target.process.clone();
                match bigame_core::fg::save_for_game(&process, 1, 100, false, false, 1, true) {
                    Ok(()) => page.overlay.add_toast(adw::Toast::new(&i18n(
                        "lsfg-vk was turned off for this game",
                    ))),
                    Err(e) => page.overlay.add_toast(adw::Toast::new(&format!(
                        "{}: {}",
                        i18n("Could not turn lsfg-vk off"),
                        error_text(&e)
                    ))),
                }
                refresh(&page);
            },
        );
    }
    {
        let page = Rc::clone(page);
        c.lsfg_conflict
            .add_action(&i18n("Use %s").replace("%s", "lsfg-vk"), true, move |_| {
                page.quiet.set(true);
                page.choice.frame_gen.set_selected(0);
                page.quiet.set(false);
                choose_frame_generation(&page, false);
            });
    }
    {
        let page = Rc::clone(page);
        c.setting_off
            .add_action(&i18n("Switch it back on"), true, move |_| {
                let page = Rc::clone(&page);
                glib::spawn_future_local(async move {
                    if refuse_while_running(&page, &page.overlay) {
                        return;
                    }
                    busy(&page, Some(&i18n("Writing the game's settings…")));
                    let target = page.target.clone();
                    let result = gio::spawn_blocking(move || {
                        graphics::switch_game_setting_on_again(&target)
                    })
                    .await;
                    busy(&page, None);
                    let text = match result {
                        Ok(Ok(_)) => i18n("Switched back on in the game's settings"),
                        Ok(Err(e)) => {
                            format!("{}: {}", i18n("Nothing was changed"), error_text(&e))
                        }
                        Err(_) => i18n("Nothing was changed"),
                    };
                    page.overlay.add_toast(adw::Toast::new(&text));
                    refresh(&page);
                });
            });
    }
    {
        let page = Rc::clone(page);
        c.wine_fsr.add_action(
            &i18n("Turn Wine FSR off for this game"),
            true,
            move |_| {
                let page = Rc::clone(&page);
                glib::spawn_future_local(async move {
                    let process = page.target.process.clone();
                    let result = gio::spawn_blocking(move || {
                        bigame_core::steam_gamescope::set_wine_fsr_off(&process, true)
                    })
                    .await;
                    let text = match result {
                        Ok(Ok(bigame_core::steam_gamescope::Applied::SteamRunning)) => i18n(
                            "Close Steam first: it keeps its launch options in memory and would overwrite the change.",
                        ),
                        Ok(Ok(bigame_core::steam_gamescope::Applied::Written(o))) => {
                            i18n("Steam launch options: %s").replace("%s", &o)
                        }
                        Ok(Err(e)) => format!("{}: {}", i18n("Nothing was changed"), error_text(&e)),
                        Ok(Ok(_)) | Err(_) => i18n("Nothing was changed"),
                    };
                    page.overlay.add_toast(adw::Toast::new(&text));
                    refresh(&page);
                });
            },
        );
    }
    {
        let page = Rc::clone(page);
        c.failure.add_action(&i18n("Diagnose"), true, move |_| {
            if let Some(d) = page.diagnose.borrow().as_ref() {
                d.set_expanded(true);
                d.grab_focus();
            }
        });
    }
}

/// Whether Wine FSR would run next to `OptiScaler` in this Steam game: on in
/// Tuning or in its own launch options, and not switched off for it by
/// BiGame-mode. Blocking (it reads Steam's configuration).
fn wine_fsr_second(target: &Target) -> bool {
    target.app_id.is_some()
        && !bigame_core::game_settings::load(&target.process).is_ok_and(|s| s.steam_wine_fsr_off)
        && (bigame_core::video_config::load().upscaling.wine_fsr_enabled
            || bigame_core::steam_gamescope::wine_fsr_in_options(&target.process))
}

/// Switch `OptiScaler`'s frame generation on or off in the choice. It is
/// experimental, so choosing it allows experimental options too.
fn choose_frame_generation(page: &Rc<Page>, on: bool) {
    {
        let mut cfg = page.cfg.borrow_mut();
        if on {
            cfg.mode = Mode::Advanced;
            cfg.frame_generation = FrameGeneration::OptiScaler;
            cfg.experimental = true;
        } else {
            cfg.frame_generation = FrameGeneration::Off;
            if page.choice.upscaler.selected() == 0 {
                cfg.mode = Mode::Recommended;
            }
        }
    }
    if on && !page.choice.experimental.is_active() {
        page.quiet.set(true);
        page.choice.experimental.set_active(true);
        page.quiet.set(false);
    }
    refresh(page);
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

/// Analyse again and redraw. Choices made quickly start several analyses;
/// only the latest one's answer is shown, so an older result arriving late
/// never overwrites a newer choice.
fn refresh(page: &Rc<Page>) {
    let page = page.clone();
    let generation = page.generation.get() + 1;
    page.generation.set(generation);
    glib::spawn_future_local(async move {
        busy(&page, Some(&i18n("Looking at the game…")));
        let target = page.target.clone();
        let cfg = page.cfg.borrow().clone();
        let analysis = gio::spawn_blocking(move || {
            let a = graphics::analyze(&target, &cfg);
            let second = a.report.installed.is_some() && wine_fsr_second(&target);
            (a, second)
        })
        .await;
        if page.generation.get() != generation {
            return;
        }
        busy(&page, None);
        let Ok((a, wine_second)) = analysis else {
            return;
        };
        page.choice.wine_fsr.set_visible(wine_second);
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
            Ok(Err(e)) => format!("{}: {}", i18n("Not updated"), error_text(&e)),
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
pub fn open(parent: &impl IsA<gtk4::Widget>, target: Target, mode: Option<Mode>) {
    open_with(parent, target, move |cfg| {
        if let Some(m) = mode {
            cfg.mode = m;
        }
    });
}

/// Open AI Graphics for `target` with `change` made to its saved choice —
/// selected, not applied: the page says so and Apply does it.
pub fn open_to_change(
    parent: &impl IsA<gtk4::Widget>,
    target: Target,
    change: impl FnOnce(&mut AiGraphicsConfig),
) {
    open_with(parent, target, change);
}

/// Building a widget tree and wiring its three actions is linear; splitting
/// it yields helpers with a single caller, so the length lint is allowed.
#[allow(clippy::too_many_lines)]
fn open_with(
    parent: &impl IsA<gtk4::Widget>,
    target: Target,
    change: impl FnOnce(&mut AiGraphicsConfig),
) {
    tracing::info!(target: "graphics", game = %target.process, "AI Graphics page opened");
    let mut cfg = match bigame_core::game_settings::load(&target.process) {
        Ok(s) => s.ai_graphics,
        Err(e) => {
            tracing::warn!(target: "graphics", game = %target.process, error = %e,
                "the game's AI Graphics settings do not read; starting from the defaults");
            AiGraphicsConfig::default()
        }
    };
    change(&mut cfg);
    if cfg.mode == Mode::Off {
        cfg.mode = Mode::Recommended;
    }

    let dialog = adw::Dialog::builder()
        .title(i18n("AI Graphics"))
        .content_width(700)
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

    let intro = gtk4::Label::new(Some(&i18n(
        "Improve image quality and performance using technologies such as DLSS, FSR, XeSS, \
         OptiScaler and compatible neural-rendering features.",
    )));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    let now = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let choice = build_choice(&cfg);
    let plan = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let extras = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&intro);
    content.append(&now);
    content.append(&choice.group);
    content.append(&plan);
    content.append(&extras);
    let clamp = adw::Clamp::builder()
        .maximum_size(700)
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
    let status = Chip::new(State::Off);
    status.widget().set_visible(false);
    // The status and the busy line on one side, the buttons on the other;
    // the buttons wrap under it when a translation is long.
    let left = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    left.set_valign(gtk4::Align::Center);
    left.append(status.widget());
    left.append(&spinner);
    left.append(&busy_label);
    let buttons = gtk4::FlowBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .max_children_per_line(3)
        .column_spacing(8)
        .row_spacing(8)
        .halign(gtk4::Align::End)
        .hexpand(true)
        .build();
    for b in [&remove, &repair, &apply] {
        buttons.insert(b, -1);
    }
    // FlowBox children are focusable cells; the buttons inside are what
    // Tab should reach.
    let mut child = buttons.first_child();
    while let Some(c) = child {
        c.set_focusable(false);
        child = c.next_sibling();
    }
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    actions.set_margin_top(6);
    actions.set_margin_bottom(12);
    actions.set_margin_start(12);
    actions.set_margin_end(12);
    actions.append(&left);
    actions.append(&buttons);

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
        scroll,
        now,
        choice,
        plan,
        extras,
        versions: RefCell::new(None),
        diagnose: RefCell::new(None),
        overlay: overlay.clone(),
        apply: apply.clone(),
        repair: repair.clone(),
        remove: remove.clone(),
        spinner,
        busy_label,
        status,
        quiet: Cell::new(false),
        generation: Cell::new(0),
        upscaler_touched: Cell::new(false),
        now_rows: RefCell::new(Vec::new()),
    });
    wire_choice(&page);
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
                        Ok(Err(e)) => format!("{}: {}", i18n("Nothing was changed"), error_text(&e)),
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
                    let done = if a.pending_changes {
                        graphics::reinstall(&target, &a.plan, &cfg.version)
                    } else {
                        graphics::install(&target, &a.plan, &cfg.version)
                    }?;
                    // Two frame generators never run together — not even
                    // for a launch BiGame-mode does not make (Steam's): with
                    // OptiScaler generating frames, the game's lsfg-vk entry
                    // goes, in lsfg-vk's own file.
                    let lsfg_removed = cfg.optiscaler_frame_generation()
                        && bigame_core::fg::read_profile_any(&target.process).0 > 1
                        && bigame_core::fg::save_for_game(&target.process, 1, 100, false, false, 1, true)
                            .is_ok();
                    // And one upscaler: Wine FSR off for this game in its
                    // Steam launch options, when Steam is closed (the page
                    // offers it otherwise).
                    if wine_fsr_second(&target) {
                        let _ = bigame_core::steam_gamescope::set_wine_fsr_off(&target.process, true);
                    }
                    anyhow::Ok((done, lsfg_removed))
                })
                .await;
                busy(&page, None);
                let text = match result {
                    Ok(Ok((done, lsfg_removed))) => {
                        use bigame_core::graphics::ingame::Applied;
                        let mut files = format!(
                            "{} ({})",
                            i18n("Installed; every replaced file was backed up"),
                            ni18n("%n file", "%n files", done.manifest.entries.len())
                        );
                        if lsfg_removed {
                            let _ = write!(files, " · {}", i18n("lsfg-vk was turned off for this game"));
                        }
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
                    Ok(Err(e)) => format!("{}: {}", i18n("Could not apply"), error_text(&e)),
                    Err(_) => i18n("Could not apply"),
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
                    Ok(Err(e)) => format!("{}: {}", i18n("Could not repair"), error_text(&e)),
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
                        Ok(Err(e)) => format!("{}: {}", i18n("Could not remove it"), error_text(&e)),
                        Err(_) => i18n("Could not remove it"),
                    };
                    overlay.add_toast(adw::Toast::new(&text));
                    refresh(&page);
                    return;
                }
                busy(&page, Some(&i18n("Restoring the game's own files…")));
                let target = page.target.clone();
                let result = gio::spawn_blocking(move || {
                    let out = graphics::remove(&target)?;
                    // BiGame-mode's WINE_FULLSCREEN_FSR=0 went in with
                    // OptiScaler, and goes with it (Steam closed; otherwise
                    // it stays, harmless, until the next Restore).
                    if bigame_core::game_settings::load(&target.process)
                        .is_ok_and(|s| s.steam_wine_fsr_off)
                    {
                        let _ = bigame_core::steam_gamescope::set_wine_fsr_off(&target.process, false);
                    }
                    anyhow::Ok(out)
                })
                .await;
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
                    Ok(Err(e)) => format!("{}: {}", i18n("Could not restore"), error_text(&e)),
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
                    Ok(Err(e)) => {
                        format!("{}: {}", i18n("Could not write the report"), error_text(&e))
                    }
                    Err(_) => i18n("Could not write the report"),
                };
                overlay.add_toast(adw::Toast::new(&text));
            });
        });
    }

    refresh(&page);
    // The game starting or closing changes what the page says (Current,
    // the status, Diagnose): read it again while the page is open. The
    // listener holds the page weakly and goes once the page is gone, and it
    // skips the call subscribe makes at once (refreshed just above).
    {
        let weak = Rc::downgrade(&page);
        let first = std::cell::Cell::new(true);
        crate::game_watch::subscribe(move |_| {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !first.replace(false) && page.now.is_mapped() {
                refresh(&page);
            }
            glib::ControlFlow::Continue
        });
    }
    dialog.present(Some(parent));
}
