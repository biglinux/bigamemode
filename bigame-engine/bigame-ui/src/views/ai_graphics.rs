//! AI Graphics for one game: what was found, what Big Game Mode recommends,
//! and — only when the user asks — doing it, repairing it, or undoing it.
//!
//! The page reads top to bottom as the questions a player has: can this
//! game get it, and if not why and what else works; what the game has;
//! what to choose; what Apply would change. It is a dry run until *Apply*
//! is pressed. *Save Choice* keeps the choice without installing anything,
//! as the profile editor's *Save Profile* keeps a profile. Details that most
//! people do not need (the API and how sure that is, DLL slots, versions)
//! are one (i) or one click away, not on top.

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
use bigame_core::graphics::report::{Confidence, Report};
use bigame_core::graphics::runtime::Status;
use bigame_core::graphics::scan::BuiltIn;
use bigame_core::graphics::versions::Offer;
use bigame_core::graphics::{self, Analysis, ChoiceState, Target, backend, diagnose, external};
use bigame_core::optimization::Feature;
use bigame_core::overview::State;

use crate::i18n::{error_text, i18n, ni18n, tr};
use crate::widgets::info::{self, Entry};
use crate::widgets::notice::{self, Kind, Notice};
use crate::widgets::status::Chip;

struct Page {
    target: Target,
    cfg: RefCell<AiGraphicsConfig>,
    /// The choice as it is saved for the game: *Save Choice* is offered
    /// while `cfg` differs from it.
    saved: RefCell<AiGraphicsConfig>,
    analysis: RefCell<Option<Analysis>>,
    scroll: gtk4::ScrolledWindow,
    /// The answer first: what the game can get, or why not and what else
    /// works. Rebuilt with each analysis.
    verdict: gtk4::Box,
    /// What the game has and what runs now: facts that do not depend on the
    /// choice, so the controls below them never move when a choice changes.
    facts: adw::PreferencesGroup,
    /// The rows of `facts` [`render_facts`] put there.
    fact_rows: RefCell<Vec<adw::ActionRow>>,
    /// What [`render_facts`] last showed.
    now_rows: RefCell<Vec<Fact>>,
    /// When the page last read the game, and what *Check Again* found.
    detect_row: adw::ActionRow,
    detect_button: gtk4::Button,
    /// Set by *Check Again*: the facts before, to say what changed.
    check: RefCell<Option<Vec<(String, String)>>>,
    /// The choice. Built once and never rebuilt: the control in use keeps
    /// its place and its focus while the rest of the page follows it.
    choice: Choice,
    /// What Apply would do, below the choice; rebuilt with each analysis.
    plan: gtk4::Box,
    /// Versions, neural rendering, technical details, Diagnose, support.
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
    save: gtk4::Button,
    spinner: gtk4::Spinner,
    busy_label: gtk4::Label,
    /// Whether the choice is saved, beside the buttons.
    hint: gtk4::Label,
    /// Where the whole choice stands, beside the buttons.
    status: Chip,
    /// Set while the page moves a control itself.
    quiet: Cell<bool>,
    /// Counts analyses, so only the latest one is shown.
    generation: Cell<u64>,
    /// The upscaler was changed since the last apply.
    upscaler_touched: Cell<bool>,
}

/// The choice rows and their state.
struct Choice {
    group: adw::PreferencesGroup,
    upscaler: adw::ComboRow,
    frame_gen: adw::ComboRow,
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

/// One fact about the game: a name, what it is, and what its (i) says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fact {
    title: String,
    value: String,
    info: Option<String>,
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
        Status::SettingsLeft => i18n(
            "Files put back; the game's own setting is still to be put back — press Restore again",
        ),
        Status::Moved { installed_in } => format!(
            "{}: {}",
            i18n("Installed in another folder"),
            installed_in.display()
        ),
        Status::Unreadable { error } => format!(
            "{}: {}",
            i18n("The record of what was installed cannot be read"),
            tr(error)
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

/// A group title that is a sentence, not a heading: let it wrap instead of
/// ending in an ellipsis (libadwaita ellipsizes group titles), as it would
/// in the Gamer theme's larger heading and in Portuguese.
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

/// Start a sentence with a capital: core writes steps as clauses ("choose
/// `XeSS` in the game's menu"), and a row title reads as a sentence.
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

fn step_icon(step: &Step) -> &'static str {
    match step {
        Step::InGame(_) => "input-gaming-symbolic",
        Step::Install(_) => "folder-download-symbolic",
        Step::Disable(_) => "action-unavailable-symbolic",
        Step::Keep(_) => "object-select-symbolic",
        Step::Note(_) => "dialog-information-symbolic",
        Step::Instead(_) => "go-next-symbolic",
    }
}

fn step_row(step: &Step) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(sentence(&tr(step.text())))
        .use_markup(false)
        .build();
    r.set_title_lines(0);
    r.add_prefix(&gtk4::Image::from_icon_name(step_icon(step)));
    r
}

/// A group of `steps`, or `None` when there are none.
fn steps_group(
    title: &str,
    description: Option<&str>,
    steps: &[&Step],
) -> Option<adw::PreferencesGroup> {
    if steps.is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(title);
    group.set_description(description);
    for s in steps {
        group.add(&step_row(s));
    }
    Some(group)
}

/// A caption-sized badge in `class`'s colour.
fn badge(text: &str, class: &str) -> gtk4::Label {
    let b = gtk4::Label::new(Some(text));
    b.add_css_class(class);
    b.add_css_class("caption-heading");
    b.set_valign(gtk4::Align::Center);
    b.set_wrap(true);
    b.set_justify(gtk4::Justification::Right);
    b.set_max_width_chars(36);
    b
}

/// Put `a` on screen: the verdict, the facts, the state of the choice, the
/// plan and the extras. The choice rows stay; what is above them depends
/// only on the game and the plan, and the view is kept in place while it
/// changes.
fn render(page: &Rc<Page>, a: &Analysis, first: bool) {
    let all = || {
        render_verdict(page, a);
        render_facts(page, a);
        render_choice_state(page, a);
        render_plan(page, a);
        render_extras(page, a);
        render_buttons(page, a);
    };
    // The first reading opens the page at its top, with the verdict: there
    // is no place to keep yet.
    if first {
        all();
        // The dialog focused its first button (Check Again) while the page
        // was empty, and the view followed it down when the verdict came in
        // above: back to the top once the new rows have their size.
        let frames = Cell::new(0u8);
        page.scroll.add_tick_callback(move |s, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            s.vadjustment().set_value(0.0);
            glib::ControlFlow::Break
        });
    } else {
        keep_in_place(&page.scroll, page.choice.group.upcast_ref(), all);
    }
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

// ── The verdict ─────────────────────────────────────────────────────────────

/// What Apply can do for this game, in one card: the plan's summary, its
/// standing, and one sentence. When there is nothing to apply, why — every
/// reason the plan gives — and what the player can use instead, right under
/// it: a status word alone does not say what to do.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn render_verdict(page: &Rc<Page>, a: &Analysis) {
    clear(&page.verdict);
    let r = &a.report;
    let p = &a.plan;
    let installed = r.installed.is_some();
    let installs = p.optiscaler.is_some() || p.native_action.is_some();
    let own_upscaler = !installs && !installed && p.standing == Standing::Recommended;
    let (icon, text) = if p.standing == Standing::Blocked {
        (
            "action-unavailable-symbolic",
            i18n("AI Graphics cannot be turned on for this game. The reason is below."),
        )
    } else if installed && p.optiscaler.is_none() {
        (
            "emblem-ok-symbolic",
            i18n(
                "Big Game Mode installed OptiScaler in this game, and with the choice below it is not needed. Restore puts the game's own files back.",
            ),
        )
    } else if a.pending_changes {
        (
            "document-edit-symbolic",
            i18n(
                "Your choice differs from what is installed. Apply changes puts the game's own files back and installs this instead.",
            ),
        )
    } else if installed {
        (
            "emblem-ok-symbolic",
            i18n(
                "What Big Game Mode installed for this game. Restore puts the game's own files back.",
            ),
        )
    } else if installs {
        (
            "emblem-ok-symbolic",
            i18n("What Big Game Mode would do. Nothing changes until you press Apply."),
        )
    } else if own_upscaler {
        (
            "input-gaming-symbolic",
            i18n(
                "Nothing to install: the game's own upscaler is the best choice here. Turn it on in the game's graphics menu, as the steps below say.",
            ),
        )
    } else {
        (
            "dialog-information-symbolic",
            i18n(
                "AI Graphics has nothing to install in this game — nothing is wrong with your system. Why, and what works instead, is below.",
            ),
        )
    };
    let image = gtk4::Image::from_icon_name(icon);
    image.set_pixel_size(32);
    image.set_valign(gtk4::Align::Center);
    image.add_css_class("scope-icon");
    let heading = gtk4::Label::builder()
        .label(sentence(&tr(&p.summary)))
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["title-4"])
        .build();
    let body = gtk4::Label::builder()
        .label(&text)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let (standing, class) = standing_text(p.standing);
    let standing_line = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    standing_line.set_margin_top(4);
    let standing_label = gtk4::Label::new(Some(&i18n("Standing")));
    standing_label.add_css_class("caption");
    standing_line.append(&standing_label);
    standing_line.append(&badge(&standing, class));
    let words = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.append(&heading);
    words.append(&body);
    words.append(&standing_line);
    let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
    card.add_css_class("scope-card");
    card.append(&image);
    card.append(&words);
    let how = decided_button();
    how.set_valign(gtk4::Align::Start);
    card.append(&how);
    page.verdict.append(&card);

    if p.native_action == Some(NativeAction::Fsr4Upgrade) && !a.fsr4_upgrade_set {
        page.verdict.append(
            Notice::new(
                Kind::Info,
                "FSR 4",
                &i18n("FSR 4 available with the launch option FSR4_UPGRADE=1"),
            )
            .widget(),
        );
    }
    // Nothing to apply and nothing installed: the reasons and the ways
    // around it are the answer, so they sit with it.
    if !installs && !installed && !own_upscaler {
        let notes: Vec<&Step> = p
            .steps
            .iter()
            .filter(|s| matches!(s, Step::Note(_)))
            .collect();
        if let Some(g) = steps_group(&i18n("Why"), None, &notes) {
            page.verdict.append(&g);
        }
        let instead: Vec<&Step> = p
            .steps
            .iter()
            .filter(|s| matches!(s, Step::InGame(_) | Step::Instead(_)))
            .collect();
        if let Some(g) = steps_group(
            &i18n("What you can do instead"),
            Some(&i18n(
                "These are set outside AI Graphics: in the game's own menu, or in its profile.",
            )),
            &instead,
        ) {
            page.verdict.append(&g);
        }
    }
    for problem in &p.problems {
        let n = Notice::new(
            Kind::Conflict,
            &format!("{} + {}", i18n(problem.a.label()), i18n(problem.b.label())),
            &sentence(&i18n(problem.why)),
        );
        page.verdict.append(n.widget());
    }
}

/// The (i) that says how the recommendation is made and what its standings
/// mean.
fn decided_button() -> gtk4::Button {
    info::dialog_button(&i18n("How this is decided"), || {
        (
            i18n("How this is decided"),
            i18n(
                "Big Game Mode picks the fewest components that give the best result for this game on \
                 this GPU. If the game's own upscaler is already the best, nothing is installed. \
                 OptiScaler is used where it adds something the game lacks — FSR 4 on RDNA 4 \
                 graphics cards — or where Big Game Mode's game list records it as faster for that \
                 game, measured on its test machines. DLSS is offered only \
                 on NVIDIA RTX cards. Frame generation is never switched on by itself: it raises the \
                 presented frame rate, not the rendered one, and adds latency. Games with \
                 anti-cheat get no injection at all.",
            ),
            vec![
                Entry {
                    title: i18n("Recommended"),
                    body: i18n(
                        "The game's own feature, or a combination verified to work on Big Game Mode's test machines.",
                    ),
                },
                Entry {
                    title: i18n("Compatible — not yet verified in practice"),
                    body: i18n(
                        "OptiScaler's documentation says it works; Big Game Mode has not verified it yet.",
                    ),
                },
                Entry {
                    title: i18n("Experimental"),
                    body: i18n(
                        "Reported to work but not established, or it depends on reporting another GPU to the game (spoofing). Try it, and Restore Game Graphics if the game does not behave.",
                    ),
                },
                Entry {
                    title: i18n("Not recommended"),
                    body: i18n(
                        "Nothing worth installing, or it would be worse than what the game already has. The reason is on the page.",
                    ),
                },
                Entry {
                    title: i18n("Blocked"),
                    body: i18n(
                        "Anti-cheat or Big Game Mode's game list: injecting a DLL could put the account at risk, so nothing is offered.",
                    ),
                },
            ],
        )
    })
}

// ── What the game has ───────────────────────────────────────────────────────

/// What the game has and runs on, a row per question a person asks in
/// order: the GPU, the API, the upscalers, what starts, what runs now.
fn render_facts(page: &Rc<Page>, a: &Analysis) {
    let r = &a.report;
    let mut facts: Vec<Fact> = Vec::new();
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
        facts.push(Fact {
            title: bigame_core::graphics::report::display_name(&g.name),
            value: sub,
            info: None,
        });
    }
    facts.push(Fact {
        title: i18n("Game API"),
        value: api_line(r),
        info: Some(api_info(r)),
    });
    let (value, about) = shipped(r);
    facts.push(Fact {
        title: i18n("Upscalers in the game"),
        value,
        info: Some(about),
    });
    if let (Some(stub), Some(exe)) = (&r.launcher_stub, &r.executable) {
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        facts.push(Fact {
            title: i18n("How the game starts"),
            value: i18n("%s starts %s")
                .replacen("%s", &stub.display().to_string(), 1)
                .replacen("%s", &name, 1),
            info: Some(i18n(
                "The launcher starts a small Unreal Engine bootstrap, which starts the real game. Big Game Mode looks at the game that runs — its folder is where OptiScaler would go, and its executable says which upscalers the game has — and knows it as running by either name.",
            )),
        });
    }
    facts.push(Fact {
        title: i18n("Running now"),
        value: running_now(a),
        info: None,
    });
    // Rebuilt only when it says something else: a change of choice leaves
    // everything above the choice exactly where it is.
    if *page.now_rows.borrow() == facts {
        return;
    }
    for old in page.fact_rows.borrow_mut().drain(..) {
        page.facts.remove(&old);
    }
    page.facts.remove(&page.detect_row);
    let mut rows = Vec::new();
    for f in &facts {
        let r = row(&f.title, &f.value);
        r.set_subtitle_lines(0);
        if let Some(text) = &f.info {
            r.add_suffix(&info::button(&f.title, text));
        }
        page.facts.add(&r);
        rows.push(r);
    }
    page.facts.add(&page.detect_row);
    *page.fact_rows.borrow_mut() = rows;
    *page.now_rows.borrow_mut() = facts;
}

/// The game's own upscalers and frame generation, and what that means.
fn shipped(r: &Report) -> (String, String) {
    let n = &r.native;
    let version = |v: &str| {
        if v == bigame_core::graphics::report::PRESENT {
            String::new()
        } else {
            format!(" {v}")
        }
    };
    let mut own = Vec::new();
    if let Some(v) = &n.dlss {
        own.push(format!("DLSS{}", version(v)));
    }
    if let Some(v) = &n.fsr {
        own.push(format!("FSR{}", version(v)));
    }
    if let Some(v) = &n.xess {
        own.push(format!("XeSS{}", version(v)));
    }
    let built_in_fsr = n.built_in_fsr();
    if let Some(b) = built_in_fsr {
        own.push(i18n("%s, built into the game").replace("%s", b.label()));
    }
    let dll_missing: Vec<BuiltIn> = n
        .built_in
        .iter()
        .copied()
        .filter(|b| !b.runs_without_a_dll())
        .collect();
    for b in &dll_missing {
        own.push(i18n("%s code, without its DLL").replace("%s", b.label()));
    }
    if n.frame_gen() {
        own.push(i18n("frame generation"));
    }
    let value = if own.is_empty() {
        i18n("None: no DLSS, FSR or XeSS")
    } else {
        own.join(" · ")
    };
    let mut about = Vec::new();
    if n.dlss.is_some() || n.fsr.is_some() || n.xess.is_some() {
        about.push(i18n(
            "Found as DLLs in the game's folder. Choose one in the game's graphics menu; OptiScaler can take over DLSS, FSR 2 or newer and XeSS.",
        ));
    }
    if let Some(b) = built_in_fsr {
        about.push(
            i18n(
                "%s is compiled into the game's executable instead of shipped as a DLL: turn it on in the game's own graphics menu. OptiScaler takes over an upscaler through the calls the game makes to it; with a built-in FSR that works in some games and not in others, depending on how the game built it — OptiScaler's compatibility list says which. Nothing else can replace it from outside the game.",
            )
            .replace("%s", b.label()),
        );
    }
    if !dll_missing.is_empty() {
        about.push(i18n(
            "DLSS and XeSS always run from their DLL (nvngx_dlss.dll, libxess.dll). The executable has code that calls one, but its DLL is not in the game's folder, so the game cannot offer it and OptiScaler has nothing to take over.",
        ));
    }
    if about.is_empty() {
        about.push(i18n(
            "The game has no DLSS, FSR or XeSS — neither as a DLL in its folder nor built into its executable. OptiScaler needs one of them to take over, so AI Graphics cannot upscale this game. Gamescope's or Wine's FSR 1 still scales the finished image from outside it.",
        ));
    }
    if let Some(exe) = &r.executable {
        about.push(
            i18n("Read from %s and the files beside it.").replace("%s", &exe.display().to_string()),
        );
    }
    (value, about.join("\n\n"))
}

/// `DirectX 12 · VKD3D-Proton · Vulkan on the host`, with how sure.
fn api_line(r: &Report) -> String {
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

/// What the API is for, and the evidence behind it.
fn api_info(r: &Report) -> String {
    let mut s = i18n(
        "The graphics API the game draws with. OptiScaler is configured for it: which upscalers it can run depends on it (FSR 4 and XeSS are DirectX 12 first; DirectX 11 and Vulkan reach them through DirectX 12).",
    );
    let evidence: Vec<String> = r.api.evidence.iter().map(|e| sentence(&tr(e))).collect();
    if !evidence.is_empty() {
        let _ = write!(
            s,
            "\n\n{}: {}",
            i18n("How it is known"),
            evidence.join(" · ")
        );
    }
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
            (Some(_), Some(false)) => i18n("running without FSR4_UPGRADE=1: FSR 3.1"),
            (Some(true), _) => i18n("FSR 4 provider loaded in the running game"),
            (Some(false), _) => i18n("running without the FSR 4 provider: FSR is off in its menu"),
            (None, _) if a.fsr4_upgrade_set => i18n("FSR 4 expected through Proton's provider"),
            (None, _) => i18n("Nothing from Big Game Mode: the game's own graphics"),
        };
    }
    i18n("Nothing from Big Game Mode: the game's own graphics")
}

/// Everything *Check Again* compares, by name: what the game has, what
/// runs, neural rendering, what is installed and what the plan says.
fn checked_facts(a: &Analysis) -> Vec<(String, String)> {
    let r = &a.report;
    vec![
        (
            i18n("Executable"),
            r.executable
                .as_ref()
                .map_or_else(|| i18n("Unknown"), |e| e.display().to_string()),
        ),
        (
            i18n("Graphics card"),
            r.gpu()
                .map(|g| bigame_core::graphics::report::display_name(&g.name))
                .unwrap_or_default(),
        ),
        (i18n("Game API"), api_line(r)),
        (i18n("Upscalers in the game"), shipped(r).0),
        (i18n("Running now"), running_now(a)),
        (i18n("Neural rendering"), neural_words(&a.neural).0),
        (
            i18n("Installed by Big Game Mode"),
            r.installed.as_ref().map_or_else(
                || i18n("Nothing"),
                |m| format!("{} {}", m.source.component, m.source.version),
            ),
        ),
        (i18n("Recommendation"), sentence(&tr(&a.plan.summary))),
    ]
}

/// What changed between two [`checked_facts`], one line each.
fn changes(before: &[(String, String)], after: &[(String, String)]) -> Vec<String> {
    after
        .iter()
        .filter_map(|(name, now)| {
            let was = before.iter().find(|(n, _)| n == name).map(|(_, v)| v)?;
            (was != now).then(|| format!("{name}: {was} → {now}"))
        })
        .collect()
}

/// The time now, as the clock shows it.
fn clock() -> String {
    glib::DateTime::now_local()
        .and_then(|t| t.format("%X"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Say what *Check Again* found, on its row and in a toast.
fn report_check(page: &Page, before: &[(String, String)], a: &Analysis) {
    let found = changes(before, &checked_facts(a));
    let when = clock();
    let (subtitle, toast) = if found.is_empty() {
        (
            i18n(
                "Checked again at %s: the game's folder, the graphics card, the running game and neural rendering read as before",
            )
            .replace("%s", &when),
            i18n("Checked again: nothing changed"),
        )
    } else {
        (
            format!(
                "{}\n{}",
                i18n("Checked again at %s:").replace("%s", &when),
                found.join("\n")
            ),
            ni18n(
                "Checked again: %n thing changed",
                "Checked again: %n things changed",
                found.len(),
            ),
        )
    };
    page.detect_row.set_subtitle(&subtitle);
    page.overlay.add_toast(adw::Toast::new(&toast));
}

// ── The choice ──────────────────────────────────────────────────────────────

/// A choice's state as a short word and a sentence.
fn state_words(s: ChoiceState) -> (String, String) {
    match s {
        ChoiceState::NothingToApply => (
            i18n("No change"),
            i18n("Nothing to install for this choice"),
        ),
        ChoiceState::Selected => (i18n("Selected"), i18n("Not applied yet — press Apply")),
        ChoiceState::NeedsRestore => (
            i18n("Selected"),
            i18n("Not applied yet — Restore Game Graphics puts the game's own files back"),
        ),
        ChoiceState::Configured => (
            i18n("Configured"),
            i18n("Applied; it starts working when the game starts"),
        ),
        ChoiceState::Loaded => (
            i18n("Loaded"),
            i18n("Loaded — choose the upscaler named in the steps in the game's graphics menu"),
        ),
        ChoiceState::Active => (i18n("Active"), i18n("Working in the running game")),
        ChoiceState::Failed => (
            i18n("Failed"),
            i18n("Applied, but the game shows a problem — Diagnose says why"),
        ),
        ChoiceState::Blocked => (i18n("Blocked"), i18n("Not offered for this game")),
    }
}

/// The subtitles of the choice rows, and the version names.
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
    let (word, text) = state_words(up_state);
    c.upscaler.set_subtitle(&format!("{word} · {text}"));

    let fg_state = match (
        cfg.optiscaler_frame_generation(),
        a.installed_frame_generation,
    ) {
        (true, Some(true)) if !a.pending_changes => Some(state),
        (true, _) | (false, Some(true)) => Some(ChoiceState::Selected),
        (false, _) => None,
    };
    if let Some(s) = fg_state {
        let (word, text) = state_words(s);
        c.frame_gen.set_subtitle(&format!("{word} · {text}"));
    } else {
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
        // The value's width follows its label, which changed.
        c.version.notify("selected");
        page.quiet.set(false);
        *c.keep_version.borrow_mut() = keep;
    }
}

// ── What will change ────────────────────────────────────────────────────────

/// What Apply would do, below the choice: what to do in the game, what
/// changes on disk (with every file), and what is good to know. For a game
/// with nothing to apply the verdict already says why and what else works.
fn render_plan(page: &Rc<Page>, a: &Analysis) {
    clear(&page.plan);
    let r = &a.report;
    let p = &a.plan;
    let installed = r.installed.is_some();
    let installs = p.optiscaler.is_some() || p.native_action.is_some();
    let own_upscaler = !installs && !installed && p.standing == Standing::Recommended;
    if !(installs || installed || own_upscaler) {
        return;
    }
    let of = |f: fn(&Step) -> bool| -> Vec<&Step> { p.steps.iter().filter(|s| f(s)).collect() };
    if let Some(g) = steps_group(
        &i18n("In the game"),
        Some(&i18n("What only the game's own menu can do.")),
        &of(|s| matches!(s, Step::InGame(_))),
    ) {
        page.plan.append(&g);
    }
    if installs || installed {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("What will change"));
        wrap_title(&group);
        group.set_description(Some(&if installed && !a.pending_changes {
            i18n("What is in the game's folder now, placed by Big Game Mode.")
        } else {
            i18n("Nothing changes until you press Apply.")
        }));
        group.set_header_suffix(Some(&info::button(
            &i18n("How files are changed safely"),
            &i18n(
                "Before Apply changes anything, every file it would replace is backed up and each copy is checked by its SHA-256. A manifest records every file placed, and each one is checked again after it is written; anything that fails is rolled back at once. The OptiScaler download is checked against the SHA-256 its release publishes. Restore Game Graphics takes out only files that are still exactly what was placed and puts every backup back; a file another program changed since is left alone. Nothing changes while the game runs.",
            ),
        )));
        for s in of(|s| matches!(s, Step::Install(_) | Step::Disable(_) | Step::Keep(_))) {
            group.add(&step_row(s));
        }
        let files = adw::ExpanderRow::builder()
            .title(i18n("Files that will change"))
            .subtitle(if p.files.is_empty() {
                i18n("None")
            } else {
                ni18n("%n file", "%n files", p.files.len())
            })
            .build();
        for f in &p.files {
            files.add_row(&row(&f.display().to_string(), ""));
        }
        files.set_sensitive(!p.files.is_empty());
        group.add(&files);
        page.plan.append(&group);
    }
    if let Some(g) = steps_group(
        &i18n("Good to know"),
        None,
        &of(|s| matches!(s, Step::Note(_) | Step::Instead(_))),
    ) {
        page.plan.append(&g);
    }
}

/// Versions, neural rendering, the evidence, Diagnose and the support
/// report.
fn render_extras(page: &Rc<Page>, a: &Analysis) {
    clear(&page.extras);
    let versions = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    versions.set_visible(false);
    page.extras.append(&versions);
    *page.versions.borrow_mut() = Some(versions);
    page.extras.append(&neural_group(page, a));
    let (group, expander) = diagnose_group(a);
    group.add(&found_expander(&a.report));
    group.add(&support_row(page));
    page.extras.append(&group);
    *page.diagnose.borrow_mut() = Some(expander);
}

/// Which buttons apply, and the one word that says where things stand.
fn render_buttons(page: &Rc<Page>, a: &Analysis) {
    let r = &a.report;
    let p = &a.plan;
    let installed = r.installed.is_some();
    let option_set = a.fsr4_upgrade_set;
    // A record of an install that cannot be used (another folder, or one
    // that does not load) stops Apply: it would take Big Game Mode's own
    // files in the game for its originals.
    let unusable = matches!(a.status, Status::Moved { .. } | Status::Unreadable { .. });
    page.apply.set_visible(
        !unusable
            && (a.pending_changes
                || !installed && (p.optiscaler.is_some() || p.native_action.is_some())),
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
    render_saved(page);
}

/// *Save Choice* while the choice differs from the saved one, and the
/// sentence beside the buttons that says which it is.
fn render_saved(page: &Page) {
    let dirty = *page.cfg.borrow() != *page.saved.borrow();
    let apply = page.apply.is_visible();
    page.save.set_visible(dirty);
    // With nothing to apply, saving is the page's one action.
    if apply {
        page.save.remove_css_class("suggested-action");
    } else {
        page.save.add_css_class("suggested-action");
    }
    page.hint.set_label(&match (dirty, apply) {
        (true, true) => i18n(
            "Your choice is not saved: Save Choice keeps it for this game, Apply saves and installs it",
        ),
        (true, false) => i18n("Your choice is not saved: Save Choice keeps it for this game"),
        (false, _) => i18n("Your choice is saved for this game"),
    });
}

// ── Neural rendering ────────────────────────────────────────────────────────

/// Neural rendering's state: a word, its colour, and what it means here.
fn neural_words(s: &external::Status) -> (String, &'static str, String) {
    match s {
        external::Status::Unavailable { missing } => (
            i18n("Unavailable here — optional, nothing is wrong"),
            "dim-label",
            format!(
                "{}\n\n{}\n{}\n\n{}",
                i18n(
                    "Neural rendering is an optional extra, separate from the upscaling above: an external project, DLSS-NR-on-AMD, runs NVIDIA's DLSS neural-rendering model on AMD RDNA 3 and RDNA 4 cards, over the game's own FSR output.",
                ),
                i18n("Why it is unavailable here:"),
                missing
                    .iter()
                    .map(|m| format!("• {}: {}", i18n(m.what), tr(&m.detail)))
                    .collect::<Vec<_>>()
                    .join("\n"),
                i18n(
                    "Nothing is wrong, and nothing needs to be done: AI Graphics works fully without it, and Big Game Mode never downloads or installs it.",
                ),
            ),
        ),
        external::Status::NotInstalled => (
            i18n("Available — not installed"),
            "accent",
            i18n(
                "This game and GPU meet what DLSS-NR-on-AMD asks for, and it is not installed beside the game. It is optional: install it yourself from its official page if you want to try it, then press Check Again.",
            ),
        ),
        external::Status::Installed { .. } => (
            i18n("Installed by you — not verified until the game runs"),
            "accent",
            i18n(
                "Its files are beside the game. Whether it works is known only when the game runs and its log says so.",
            ),
        ),
        external::Status::Loaded { .. } => (
            i18n("Loaded in the game — the pass has not reported yet"),
            "accent",
            i18n("The running game has loaded it; its log has not said yet that the pass runs."),
        ),
        external::Status::Active { .. } => (
            i18n("Active"),
            "success",
            i18n("Its log, written since the game started, says the pass runs."),
        ),
        external::Status::Failed { .. } => (
            i18n("Failed"),
            "error",
            i18n(
                "Its log, written since the game started, reports an error. Its own setup removes it.",
            ),
        ),
        external::Status::Blocked { .. } => (
            i18n("Not offered: this game has anti-cheat"),
            "dim-label",
            i18n("Nothing is injected into a game with anti-cheat, whatever is installed."),
        ),
    }
}

/// Neural rendering: the external backend's state, what is missing, and
/// the page to get it from. Nothing here downloads or places a file.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn neural_group(page: &Rc<Page>, a: &Analysis) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Neural rendering (optional)"));
    group.set_description(Some(&i18n(
        "An optional extra over the game's own FSR, from an external project. Not needed for anything above.",
    )));
    group.set_header_suffix(Some(&info::dialog_button(
        &i18n("About neural rendering"),
        || {
            (
                i18n("Neural rendering (optional)"),
                i18n(
                    "Not needed for the upscaling above. A neural pass over the game's own FSR output, through DLSS-NR-on-AMD — an external project Big Game Mode does not distribute, install or remove. Experimental: documented for Windows, not established under Proton.",
                ),
                vec![
                    Entry {
                        title: i18n("What it needs"),
                        body: i18n(
                            "An AMD RDNA 3 or RDNA 4 card, a 64-bit Windows game in DirectX 12 under Proton that ships AMD's FidelityFX API (FSR 3.1 or newer), and NVIDIA's neural-rendering model (nvngx_dlssnr.dll), which Big Game Mode never downloads.",
                        ),
                    },
                    Entry {
                        title: i18n("Why Big Game Mode only links to it"),
                        body: i18n(
                            "Its license allows personal use and forbids redistribution, so Big Game Mode only links to it. Install it beside the game with its own setup, then press Check Again.",
                        ),
                    },
                    Entry {
                        title: i18n("Check Again"),
                        body: i18n(
                            "After installing or removing it with its own setup, Check Again reads the game's folder again and says what changed.",
                        ),
                    },
                ],
            )
        },
    )));
    let (status, class, meaning) = neural_words(&a.neural);
    let status_row = adw::ActionRow::builder()
        .title(i18n("Status"))
        .subtitle("DLSS-NR-on-AMD")
        .use_markup(false)
        .build();
    status_row.add_suffix(&badge(&status, class));
    status_row.add_suffix(&info::button(
        &i18n("Neural rendering (optional)"),
        &meaning,
    ));
    group.add(&status_row);

    match &a.neural {
        external::Status::Unavailable { missing } => {
            for m in missing {
                let r = row(&i18n(m.what), &tr(&m.detail));
                r.set_subtitle_lines(0);
                r.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
                group.add(&r);
            }
        }
        external::Status::NotInstalled => {
            let r = adw::ActionRow::builder()
                .title(i18n("Get it from its official page"))
                .subtitle(i18n(
                    "Its license allows personal use and forbids redistribution, so Big Game Mode only links to it. Install it beside the game with its own setup, then press Check Again.",
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
                parts.push(i18n("model %s").replace("%s", m));
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
    let again = gtk4::Button::builder()
        .label(i18n("Check Again"))
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n(
            "Read the game's folder, the graphics card and the running game again, and say what changed",
        ))
        .build();
    {
        let page = page.clone();
        again.connect_clicked(move |_| check_again(&page));
    }
    let again_row = adw::ActionRow::builder()
        .title(i18n("After installing or removing it with its own setup"))
        .use_markup(false)
        .build();
    again_row.set_title_lines(0);
    again_row.add_suffix(&again);
    group.add(&again_row);
    group
}

// ── Diagnose, details, support ──────────────────────────────────────────────

/// Diagnose: every check with what it found and what to do.
fn diagnose_group(a: &Analysis) -> (adw::PreferencesGroup, adw::ExpanderRow) {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Diagnose and details"));
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
            let _ = write!(sub, "\n→ {}", tr(act));
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

/// "Technical details": the evidence behind the plan, for whoever wants it.
// Linear widget building, as `open`.
#[allow(clippy::too_many_lines)]
fn found_expander(r: &Report) -> adw::ExpanderRow {
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
    for b in &n.built_in {
        native.push(i18n("%s, built into the game").replace("%s", b.label()));
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
    if let Some(stub) = &r.launcher_stub {
        details.add_row(&row(&i18n("Started through"), &stub.display().to_string()));
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
            &i18n("Installed by Big Game Mode"),
            &format!(
                "{} {} · {}",
                m.source.component,
                m.source.version,
                ni18n("%n file", "%n files", m.entries.len())
            ),
        ));
    }
    details
}

/// The support report: a labelled button, and where the file went.
fn support_row(page: &Rc<Page>) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(i18n("Support report"))
        .subtitle(i18n(
            "A zip in Downloads with what this page found, what was planned and placed, and the logs — home folder, user and host names masked. For a bug report.",
        ))
        .use_markup(false)
        .build();
    r.set_subtitle_lines(0);
    let button = gtk4::Button::builder()
        .label(i18n("Save Report"))
        .valign(gtk4::Align::Center)
        .build();
    {
        let page = page.clone();
        let row = r.clone();
        button.connect_clicked(move |b| save_report(&page, b, &row));
    }
    r.add_suffix(&button);
    r
}

/// Write the support report, and say where it is.
fn save_report(page: &Rc<Page>, button: &gtk4::Button, row: &adw::ActionRow) {
    let Some(a) = page.analysis.borrow().clone() else {
        return;
    };
    let (page, button, row) = (page.clone(), button.clone(), row.clone());
    glib::spawn_future_local(async move {
        button.set_sensitive(false);
        busy(&page, Some(&i18n("Writing the report…")));
        let target = page.target.clone();
        let dest =
            glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(glib::home_dir);
        let result = gio::spawn_blocking(move || {
            bigame_core::graphics::support::write_report(&target, &a, &dest)
        })
        .await;
        busy(&page, None);
        button.set_sensitive(true);
        match result {
            Ok(Ok(path)) => {
                let text = i18n("Report saved to %s").replace("%s", &path.display().to_string());
                row.set_subtitle(&text);
                let toast = adw::Toast::builder()
                    .title(&text)
                    .button_label(i18n("Show"))
                    .build();
                let win = button.root().and_downcast::<gtk4::Window>();
                toast.connect_button_clicked(move |_| {
                    let file = gio::File::for_path(&path);
                    gtk4::FileLauncher::new(Some(&file)).open_containing_folder(
                        win.as_ref(),
                        gio::Cancellable::NONE,
                        |_| {},
                    );
                });
                page.overlay.add_toast(toast);
            }
            Ok(Err(e)) => page.overlay.add_toast(adw::Toast::new(&format!(
                "{}: {}",
                i18n("Could not write the report"),
                error_text(&e)
            ))),
            Err(_) => page
                .overlay
                .add_toast(adw::Toast::new(&i18n("Could not write the report"))),
        }
    });
}

// ── The choice rows ─────────────────────────────────────────────────────────

/// A combo row over `items` whose every item reads in full, in the list and
/// as the value (libadwaita's own label stops at about 20 characters), with
/// an (i) that explains each item.
fn choice_row(
    title: &str,
    items: &gtk4::StringList,
    selected: u32,
    about: &gtk4::Button,
) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(items)
        .use_subtitle(false)
        .factory(&crate::widgets::resolution::whole_label_factory())
        .selected(selected)
        .build();
    crate::widgets::optimization::cap_subtitle(row.upcast_ref(), 34);
    crate::widgets::optimization::keep_value_width(&row);
    row.add_suffix(about);
    row
}

/// The (i) of the Upscaler row: each item.
fn upscaler_about() -> gtk4::Button {
    info::dialog_button(&i18n("About the upscaler choices"), || {
        (
            i18n("Upscaler"),
            i18n(
                "An upscaler renders the game at a lower resolution and rebuilds a sharp image at the display's, for a higher frame rate. The quality preset stays the one chosen in the game's own menu. Nothing is installed until you press Apply.",
            ),
            vec![
                Entry {
                    title: i18n("Recommended for this game"),
                    body: i18n(
                        "Big Game Mode picks for this game and this graphics card: the game's own upscaler when it is already the best, or OptiScaler where it adds something the game lacks (FSR 4 on RDNA 4 cards) or where Big Game Mode measured it faster. The card at the top says what that is here.",
                    ),
                },
                Entry {
                    title: i18n("The game's own only"),
                    body: i18n(
                        "Nothing is installed. The page says which of the game's own upscalers to choose in its graphics menu.",
                    ),
                },
                Entry {
                    title: "FSR (OptiScaler)".to_owned(),
                    body: i18n(
                        "OptiScaler takes over the upscaler the game has (DLSS, FSR 2 or newer, or XeSS) and runs AMD FSR in its place: FSR 4 on Radeon RX 9000 (RDNA 4) cards, FSR 3.1 on the others. Runs on every GPU. The game must have an upscaler for OptiScaler to take over.",
                    ),
                },
                Entry {
                    title: "XeSS (OptiScaler)".to_owned(),
                    body: i18n(
                        "OptiScaler runs Intel XeSS in place of the game's upscaler. Fastest on Intel Arc; other GPUs run it through a slower general path. The game must have an upscaler for OptiScaler to take over.",
                    ),
                },
            ],
        )
    })
}

/// The (i) of the Frame generation row: each item.
fn frame_gen_about() -> gtk4::Button {
    info::dialog_button(&i18n("About the frame generation choices"), || {
        (
            i18n("Frame generation"),
            i18n(
                "Frame generation shows an extra, generated frame between two rendered ones: motion looks smoother, but the game does not render more frames and responds a little later. It is never switched on by itself.",
            ),
            vec![
                Entry {
                    title: i18n("Off"),
                    body: i18n(
                        "No generated frames from OptiScaler. The game's own frame generation, if it has one, stays in its menu.",
                    ),
                },
                Entry {
                    title: i18n("OptiScaler frame generation"),
                    body: i18n(
                        "OptiScaler's own frame generation (OptiFG, with FSR frame generation), for DirectX 12 games. Experimental: the HUD can ghost, and it adds latency. Choosing it allows experimental options. Never together with lsfg-vk: two frame generators in series generate frames from generated frames.",
                    ),
                },
            ],
        )
    })
}

/// The (i) of the `OptiScaler` version row: each item.
fn version_about() -> gtk4::Button {
    info::dialog_button(&i18n("About the OptiScaler versions"), || {
        (
            i18n("OptiScaler version"),
            i18n("Used for the next install; an installed game is updated only when you choose"),
            vec![
                Entry {
                    title: i18n("Tested with Big Game Mode"),
                    body: i18n(
                        "The release Big Game Mode was tested with, checked against the SHA-256 it records. The default.",
                    ),
                },
                Entry {
                    title: i18n("Latest stable"),
                    body: i18n(
                        "The newest stable release on OptiScaler's GitHub page, never older than the tested one. Taken only with a single archive and a published SHA-256, which the download is checked against.",
                    ),
                },
                Entry {
                    title: i18n("Keep one version"),
                    body: i18n(
                        "Stay on one version — the installed one, or the tested one — until you change it. Newer releases are not offered for this game.",
                    ),
                },
            ],
        )
    })
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
    let upscaler = choice_row(
        &i18n("Upscaler"),
        &ups,
        match (cfg.mode, cfg.layer, cfg.upscaler) {
            (Mode::Advanced, Layer::Native, _) => 1,
            (Mode::Advanced, _, Upscaler::Fsr) => 2,
            (Mode::Advanced, _, Upscaler::Xess) => 3,
            _ => 0,
        },
        &upscaler_about(),
    );
    group.add(&upscaler);

    let fgs = gtk4::StringList::new(&[&i18n("Off"), &i18n("OptiScaler frame generation")]);
    let frame_gen = choice_row(
        &i18n("Frame generation"),
        &fgs,
        u32::from(cfg.optiscaler_frame_generation()),
        &frame_gen_about(),
    );
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
            "OptiScaler upscales it, and Wine FSR would scale the image a second time whenever the game runs fullscreen below the display's resolution. Big Game Mode's own launch turns it off; the Steam client needs it in the game's launch options.",
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
    experimental.add_suffix(&info::button(
        &i18n("Allow experimental options"),
        &i18n(
            "Experimental combinations are reported to work but not established: OptiScaler's frame generation, and taking over DLSS in a game that hides it on this GPU (which needs OptiScaler to report an NVIDIA GPU). They are installed with the same backups as any other, and Restore Game Graphics undoes them.",
        ),
    ));
    advanced.add_row(&experimental);
    let tested = bigame_core::graphics::optiscaler::Release::recommended().version;
    let keep = match &cfg.version {
        VersionPolicy::Pinned(v) => v.clone(),
        _ => tested.clone(),
    };
    let versions = gtk4::StringList::new(&[
        &format!("{} ({tested})", i18n("Tested with Big Game Mode")),
        &i18n("Latest stable"),
        &format!("{} ({keep})", i18n("Keep one version")),
    ]);
    let version = choice_row(
        &i18n("OptiScaler version"),
        &versions,
        match cfg.version {
            VersionPolicy::Recommended => 0,
            VersionPolicy::Latest => 1,
            VersionPolicy::Pinned(_) => 2,
        },
        &version_about(),
    );
    version.set_subtitle(&i18n(
        "Used for the next install; an installed game is updated only when you choose",
    ));
    advanced.add_row(&version);
    group.add(&advanced);
    Choice {
        group,
        upscaler,
        frame_gen,
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
            let policy = match r.selected() {
                1 => VersionPolicy::Latest,
                2 => VersionPolicy::Pinned(keep),
                _ => VersionPolicy::Recommended,
            };
            save_version_choice(&page, |c| c.version = policy.clone());
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
                    if refuse_while_running(&page, &page.overlay).await {
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
/// Tuning, in a Turbo preset, or in its own launch options, and not switched
/// off for it by Big Game Mode. Blocking (it reads Steam's configuration).
fn wine_fsr_second(target: &Target) -> bool {
    target.app_id.is_some()
        && !bigame_core::game_settings::load(&target.process).is_ok_and(|s| s.steam_wine_fsr_off)
        && (bigame_core::video_config::load().upscaling.wine_fsr_enabled
            || preset_turns_wine_fsr_on()
            || bigame_core::steam_gamescope::wine_fsr_in_options(&target.process))
}

/// A Turbo preset that switches Wine FSR on reaches every game the Steam
/// client starts, through the session's environment: More FPS did, in Shadow
/// of the Tomb Raider with `OptiScaler` installed. The preset in force and the
/// one chosen for the next Turbo both count, since Turbo can be switched on
/// after `OptiScaler` is installed without this page being opened again.
fn preset_turns_wine_fsr_on() -> bool {
    use bigame_core::turbo_preset as preset;
    preset::active_levers().wine_fsr == Some(true)
        || preset::levers(preset::chosen(), preset::Machine::detect()).wine_fsr == Some(true)
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
    page.hint.set_visible(!on);
    for b in [
        &page.apply,
        &page.repair,
        &page.remove,
        &page.save,
        &page.detect_button,
    ] {
        b.set_sensitive(!on);
    }
}

/// *Check Again*: read the game's folder, the GPU, the running game and
/// neural rendering again — nothing is kept from the last reading but the
/// choice — and say what changed.
fn check_again(page: &Rc<Page>) {
    let before = page
        .analysis
        .borrow()
        .as_ref()
        .map(checked_facts)
        .unwrap_or_default();
    *page.check.borrow_mut() = Some(before);
    page.detect_row
        .set_subtitle(&i18n("Reading the game again…"));
    refresh(page);
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
            if page.check.borrow_mut().take().is_some() {
                page.detect_row
                    .set_subtitle(&i18n("The game could not be read again"));
            }
            return;
        };
        page.choice.wine_fsr.set_visible(wine_second);
        let installed = a.report.installed.is_some();
        let first = page.analysis.borrow().is_none();
        // Stored first: the page reads it while it is built.
        *page.analysis.borrow_mut() = Some(a.clone());
        render(&page, &a, first);
        let check = page.check.borrow_mut().take();
        if let Some(before) = check {
            report_check(&page, &before, &a);
        } else if first {
            page.detect_row.set_subtitle(
                &i18n("Read at %s from the game's folder, the graphics card and the running game")
                    .replace("%s", &clock()),
            );
        }
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
        &i18n("Installed version %s").replace("%s", &offer.installed),
        &if pinned {
            i18n("Kept at this version: newer releases are not offered")
        } else {
            i18n("Newer releases are offered here; nothing is updated by itself")
        },
    ));
    if let Some(new) = &offer.available {
        let r = adw::ActionRow::builder()
            .title(i18n("Update available: %s").replace("%s", &new.version))
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
                save_version_choice(&page, |c| c.skipped_update = Some(v.clone()));
                refresh(&page);
            });
        }
        {
            let (page, v) = (page.clone(), offer.installed.clone());
            keep.connect_clicked(move |_| {
                save_version_choice(&page, |c| c.version = VersionPolicy::Pinned(v.clone()));
                refresh(&page);
            });
        }
    }
    if let Some(prev) = &offer.previous {
        let r = row(
            &i18n("Before the last update: %s").replace("%s", prev),
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
        if refuse_while_running(&page, &page.overlay).await {
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

/// Say what switching the FSR 4 upgrade `on` or off for the game whose
/// process is `process` did; a Heroic that is open and runs no game is
/// offered to be closed for it.
fn report_fsr4_upgrade(
    overlay: &adw::ToastOverlay,
    process: &str,
    on: bool,
    result: Result<anyhow::Result<graphics::fsr4_upgrade::Applied>, Box<dyn std::any::Any + Send>>,
) {
    use graphics::fsr4_upgrade::Applied;
    let text = match result {
        Ok(Ok(Applied::SteamLaunchOptions(o))) if on => {
            format!(
                "{}: {o}",
                i18n("Steam's launch options for this game now read")
            )
        }
        Ok(Ok(Applied::SteamLaunchOptions(_))) => {
            i18n("The launch option was removed; the game's own FSR runs as it did")
        }
        Ok(Ok(Applied::SteamRunning)) => {
            i18n("Close Steam first: it would discard the launch option")
        }
        Ok(Ok(Applied::Heroic)) if on => i18n(
            "Written into Heroic's settings for this game: PROTON_FSR4_UPGRADE=1 (GE-Proton) and FSR4_UPGRADE=1 (Valve's Proton)",
        ),
        Ok(Ok(Applied::Heroic)) => {
            i18n("Taken out of Heroic's settings for this game; the game's own FSR runs as it did")
        }
        Ok(Ok(Applied::HeroicRunning {
            game_running: true, ..
        })) => i18n(
            "Heroic is running a game: close the game first. Heroic keeps this game's settings in memory and would discard the change.",
        ),
        Ok(Ok(Applied::HeroicRunning { launcher, .. })) => {
            let process = process.to_owned();
            crate::widgets::optimization::offer_close_heroic(
                overlay,
                launcher,
                &i18n(
                    "Heroic is open: it keeps this game's settings in memory and would discard the change",
                ),
                move || {
                    bigame_core::optimization::set_heroic_fsr4_upgrade(&process, on).map(|_| ())
                },
            );
            return;
        }
        Ok(Ok(Applied::LaunchPlan)) => {
            i18n("Not a Steam game: the variable goes into Big Game Mode's own launch")
        }
        Ok(Err(e)) => format!("{}: {}", i18n("Nothing was changed"), error_text(&e)),
        Err(_) => i18n("Nothing was changed"),
    };
    overlay.add_toast(adw::Toast::new(&text));
}

/// Files cannot change while the game runs (its DLLs are loaded, and a
/// change takes effect only at the next start). Checked here, in the UI's
/// language, before core's own check would refuse in English.
async fn refuse_while_running(page: &Page, overlay: &adw::ToastOverlay) -> bool {
    // A /proc walk, and the first time the game's executable is read: off the
    // main thread. A check that could not run refuses, as a running game does.
    let target = page.target.clone();
    let running = gio::spawn_blocking(move || graphics::is_running(&target))
        .await
        .unwrap_or(true);
    if running {
        overlay.add_toast(adw::Toast::new(&i18n(
            "Close the game first: its files are in use, and a change takes effect at the next start",
        )));
        return true;
    }
    false
}

/// Save the choice for the game — a file in the user's own configuration,
/// no game file and no privilege — and remember it as saved. Says so when
/// it cannot be written.
/// Save one version decision (Skip, Keep this version) on top of what is
/// saved, and make it in the pending choice too. The rest of a choice not
/// saved yet stays pending, as the hint beside the buttons says: these
/// buttons answer the update offer, not *Save Choice*.
fn save_version_choice(page: &Page, change: impl Fn(&mut AiGraphicsConfig)) -> bool {
    change(&mut page.cfg.borrow_mut());
    let mut saved = page.saved.borrow().clone();
    change(&mut saved);
    let result = bigame_core::game_settings::load(&page.target.process).and_then(|mut s| {
        s.ai_graphics = saved.clone();
        bigame_core::game_settings::save(&page.target.process, &s)
    });
    match result {
        Ok(()) => {
            *page.saved.borrow_mut() = saved;
            render_saved(page);
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not save the AI Graphics version choice");
            page.overlay.add_toast(adw::Toast::new(&format!(
                "{}: {}",
                i18n("The choice was not saved"),
                error_text(&e)
            )));
            false
        }
    }
}

fn save_settings(page: &Page) -> bool {
    let cfg = page.cfg.borrow().clone();
    let result = bigame_core::game_settings::load(&page.target.process).and_then(|mut s| {
        s.ai_graphics = cfg.clone();
        bigame_core::game_settings::save(&page.target.process, &s)
    });
    match result {
        Ok(()) => {
            *page.saved.borrow_mut() = cfg;
            render_saved(page);
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not save AI Graphics settings");
            page.overlay.add_toast(adw::Toast::new(&format!(
                "{}: {}",
                i18n("The choice was not saved"),
                error_text(&e)
            )));
            false
        }
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

/// Building a widget tree and wiring its actions is linear; splitting it
/// yields helpers with a single caller, so the length lint is allowed.
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
    if cfg.mode == Mode::Off {
        cfg.mode = Mode::Recommended;
    }
    // What is shown on opening counts as saved: opening the page is not a
    // change. A change handed in (the wizard's, a profile's) is one.
    let saved = cfg.clone();
    change(&mut cfg);
    if cfg.mode == Mode::Off {
        cfg.mode = Mode::Recommended;
    }

    let dialog = adw::Dialog::builder()
        .title(i18n("AI Graphics"))
        .content_width(780)
        .content_height(760)
        .build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &i18n("AI Graphics"),
        &target.name,
    )));

    let intro = gtk4::Label::new(Some(&i18n(
        "Improve image quality and performance using technologies such as DLSS, FSR, XeSS, \
         OptiScaler and compatible neural-rendering features.",
    )));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.add_css_class("dim-label");
    let verdict = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let facts = adw::PreferencesGroup::new();
    facts.set_title(&i18n("What the game has"));
    facts.set_description(Some(&i18n("What the game has and what it runs now.")));
    let detect_button = gtk4::Button::builder()
        .label(i18n("Check Again"))
        .valign(gtk4::Align::Center)
        .tooltip_text(i18n(
            "Read the game's folder, the graphics card and the running game again, and say what changed",
        ))
        .build();
    let detect_row = adw::ActionRow::builder()
        .title(i18n("Last read"))
        .subtitle(i18n("Reading the game…"))
        .use_markup(false)
        .build();
    detect_row.set_subtitle_lines(0);
    detect_row.add_prefix(&gtk4::Image::from_icon_name("view-refresh-symbolic"));
    detect_row.add_suffix(&detect_button);
    facts.add(&detect_row);
    let choice = build_choice(&cfg);
    let plan = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    let extras = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&intro);
    content.append(&verdict);
    content.append(&facts);
    content.append(&choice.group);
    content.append(&plan);
    content.append(&extras);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
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
    let save = gtk4::Button::with_label(&i18n("Save Choice"));
    save.add_css_class("pill");
    save.set_tooltip_text(Some(&i18n(
        "Keep this choice for the game without installing anything",
    )));
    let repair = gtk4::Button::with_label(&i18n("Repair"));
    repair.add_css_class("pill");
    let remove = gtk4::Button::with_label(&i18n("Restore Game Graphics"));
    remove.add_css_class("destructive-action");
    remove.add_css_class("pill");
    let spinner = gtk4::Spinner::new();
    let busy_label = gtk4::Label::new(None);
    busy_label.add_css_class("dim-label");
    busy_label.add_css_class("caption");
    let hint = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["dim-label", "caption"])
        .build();
    let status = Chip::new(State::Off);
    status.widget().set_visible(false);
    // The status and the save state on one side, the buttons on the other;
    // the buttons wrap under it when a translation is long.
    let left = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    left.set_valign(gtk4::Align::Center);
    left.set_hexpand(true);
    left.append(status.widget());
    left.append(&spinner);
    left.append(&busy_label);
    left.append(&hint);
    // A wrap box rather than a FlowBox: no focusable cells around the
    // buttons, and no cells GTK measures at a width they cannot have.
    let buttons = adw::WrapBox::builder()
        .child_spacing(8)
        .line_spacing(8)
        .align(1.0)
        .halign(gtk4::Align::End)
        .build();
    for b in [&remove, &repair, &save, &apply] {
        buttons.append(b);
    }
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    actions.add_css_class("editor-save-bar");
    actions.set_margin_top(8);
    actions.set_margin_bottom(8);
    actions.set_margin_start(16);
    actions.set_margin_end(16);
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
        saved: RefCell::new(saved),
        analysis: RefCell::new(None),
        scroll,
        verdict,
        facts,
        fact_rows: RefCell::new(Vec::new()),
        now_rows: RefCell::new(Vec::new()),
        detect_row,
        detect_button: detect_button.clone(),
        check: RefCell::new(None),
        choice,
        plan,
        extras,
        versions: RefCell::new(None),
        diagnose: RefCell::new(None),
        overlay: overlay.clone(),
        apply: apply.clone(),
        repair: repair.clone(),
        remove: remove.clone(),
        save: save.clone(),
        spinner,
        busy_label,
        hint,
        status,
        quiet: Cell::new(false),
        generation: Cell::new(0),
        upscaler_touched: Cell::new(false),
    });
    wire_choice(&page);
    busy(&page, None);
    for b in [&page.apply, &page.repair, &page.remove, &page.save] {
        b.set_visible(false);
    }
    {
        let page = page.clone();
        detect_button.connect_clicked(move |_| check_again(&page));
    }
    {
        let page = page.clone();
        save.connect_clicked(move |_| {
            if save_settings(&page) {
                page.overlay.add_toast(adw::Toast::new(&if page.apply.is_visible() {
                    i18n("Choice saved for this game. Nothing is installed until you press Apply.")
                } else {
                    i18n("Choice saved for this game")
                }));
            }
        });
    }

    {
        let page = page.clone();
        let overlay = overlay.clone();
        apply.connect_clicked(move |_| {
            let page = page.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                if refuse_while_running(&page, &overlay).await {
                    return;
                }
                let native_only = page
                    .analysis
                    .borrow()
                    .as_ref()
                    .is_some_and(|a| a.plan.optiscaler.is_none() && a.plan.native_action.is_some());
                if native_only {
                    // The Native backend's one action: a Steam launch option,
                    // written with Steam closed and read back, or the
                    // variables in a Heroic game's settings, written with
                    // Heroic closed. No game file.
                    if page.target.app_id.is_some() && bigame_core::steam::is_running() {
                        overlay.add_toast(adw::Toast::new(&i18n(
                            "Close Steam first: it keeps its configuration in memory and would discard the launch option",
                        )));
                        return;
                    }
                    busy(&page, Some(&i18n("Writing the launch option…")));
                    save_settings(&page);
                    let app = page.target.app_id.clone();
                    let process = page.target.process.clone();
                    let result = gio::spawn_blocking(move || {
                        bigame_core::graphics::fsr4_upgrade::apply(&process, app.as_deref(), true)
                    })
                    .await;
                    busy(&page, None);
                    report_fsr4_upgrade(&overlay, &page.target.process, true, result);
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
                    // for a launch Big Game Mode does not make (Steam's): with
                    // OptiScaler generating frames, the game's lsfg-vk entry
                    // goes, in lsfg-vk's own file.
                    let lsfg_removed = cfg.optiscaler_frame_generation()
                        && bigame_core::fg::read_profile_any(&target.process).0 > 1
                        && bigame_core::fg::save_for_game(&target.process, 1, 100, false, false, 1, true)
                            .is_ok();
                    // And one upscaler: Wine FSR off for this Steam game in
                    // its launch options, whatever turns it on (Tuning, a
                    // Turbo preset, the options themselves). One owner
                    // writes it, and writing it again changes nothing; with
                    // Steam open nothing is written, and the page offers it.
                    if target.app_id.is_some()
                        && let Err(e) =
                            bigame_core::steam_gamescope::set_wine_fsr_off(&target.process, true)
                        {
                            tracing::warn!(target: "graphics", game = %target.process,
                                error = %format!("{e:#}"), "Wine FSR could not be turned off for the game");
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
                        if done.kept_settings > 0 {
                            let _ = write!(
                                files,
                                " · {}",
                                ni18n(
                                    "%n setting you changed in OptiScaler's overlay was kept",
                                    "%n settings you changed in OptiScaler's overlay were kept",
                                    done.kept_settings
                                )
                            );
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
                if refuse_while_running(&page, &overlay).await {
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
                if refuse_while_running(&page, &overlay).await {
                    return;
                }
                let installed = page
                    .analysis
                    .borrow()
                    .as_ref()
                    .is_some_and(|a| a.report.installed.is_some());
                if !installed {
                    if page.target.app_id.is_some() && bigame_core::steam::is_running() {
                        overlay.add_toast(adw::Toast::new(&i18n(
                            "Close Steam first: it keeps its configuration in memory and would discard the change",
                        )));
                        return;
                    }
                    busy(&page, Some(&i18n("Removing the launch option…")));
                    let app = page.target.app_id.clone();
                    let process = page.target.process.clone();
                    let result = gio::spawn_blocking(move || {
                        bigame_core::graphics::fsr4_upgrade::apply(&process, app.as_deref(), false)
                    })
                    .await;
                    busy(&page, None);
                    report_fsr4_upgrade(&overlay, &page.target.process, false, result);
                    refresh(&page);
                    return;
                }
                busy(&page, Some(&i18n("Restoring the game's own files…")));
                let target = page.target.clone();
                let result = gio::spawn_blocking(move || {
                    let out = graphics::restore(&target)?;
                    // Big Game Mode's WINE_FULLSCREEN_FSR=0 went in with
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
                    Ok(Ok(done)) => {
                        use bigame_core::graphics::transaction::FileOutcome;
                        let kept = done
                            .files
                            .iter()
                            .filter(|o| matches!(o, FileOutcome::KeptChanged(_)))
                            .count();
                        let mut text = if kept == 0 {
                            i18n("The game's files are as they were before")
                        } else {
                            format!(
                                "{} ({kept})",
                                i18n(
                                    "Restored; files another program changed since were left alone"
                                )
                            )
                        };
                        // The user's OptiScaler settings went with its ini:
                        // where the copy is.
                        for o in &done.files {
                            if let FileOutcome::EditedCopyKept(_, copy) = o {
                                let _ = write!(
                                    text,
                                    " · {}",
                                    i18n("your edited settings were kept in %s")
                                        .replace("%s", &copy.display().to_string())
                                );
                            }
                        }
                        if let Some(why) = &done.settings_error {
                            let _ = write!(
                                text,
                                " · {}: {}",
                                i18n("The game's own setting could not be put back yet"),
                                tr(why)
                            );
                        }
                        text
                    }
                    Ok(Err(e)) => format!("{}: {}", i18n("Could not restore"), error_text(&e)),
                    Err(_) => i18n("Could not restore"),
                };
                overlay.add_toast(adw::Toast::new(&text));
                refresh(&page);
            });
        });
    }

    // Closing with a choice that is not saved asks first, as a document
    // does: the choice is kept, dropped, or the page stays open.
    {
        let page = page.clone();
        dialog.connect_close_attempt(move |d| {
            if *page.cfg.borrow() == *page.saved.borrow() {
                d.force_close();
                return;
            }
            let ask = adw::AlertDialog::new(
                Some(&i18n("Save your choice?")),
                Some(&i18n(
                    "The choice for this game is not saved. Saving keeps it for the next time; nothing is installed either way.",
                )),
            );
            ask.add_responses(&[
                ("cancel", &i18n("Cancel")),
                ("discard", &i18n("Discard")),
                ("save", &i18n("Save Choice")),
            ]);
            ask.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            ask.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            ask.set_default_response(Some("save"));
            ask.set_close_response("cancel");
            let (page, d2) = (page.clone(), d.clone());
            ask.connect_response(None, move |_, response| {
                // Saving that fails keeps the page open, with the toast
                // saying why.
                let close = match response {
                    "save" => save_settings(&page),
                    "discard" => true,
                    _ => false,
                };
                if close {
                    d2.force_close();
                }
            });
            ask.present(Some(d));
        });
        dialog.set_can_close(false);
    }

    refresh(&page);
    // The game starting or closing changes what the page says (Current,
    // the status, Diagnose): read it again while the page is open. The
    // listener goes when the dialog closes (the page's own widgets hold it,
    // so a weak reference alone would never let go), and it skips the call
    // subscribe makes at once (refreshed just above).
    {
        let closed = Rc::new(std::cell::Cell::new(false));
        dialog.connect_closed({
            let closed = Rc::clone(&closed);
            move |_| closed.set(true)
        });
        let weak = Rc::downgrade(&page);
        let first = std::cell::Cell::new(true);
        crate::game_watch::subscribe(move |_| {
            let Some(page) = weak.upgrade().filter(|_| !closed.get()) else {
                return glib::ControlFlow::Break;
            };
            if !first.replace(false) && page.verdict.is_mapped() {
                refresh(&page);
            }
            glib::ControlFlow::Continue
        });
    }
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_again_names_each_change_and_nothing_else() {
        let fact = |n: &str, v: &str| (n.to_owned(), v.to_owned());
        let before = [fact("Upscalers", "None"), fact("Running now", "Nothing")];
        let after = [
            fact("Upscalers", "FSR 2, built into the game"),
            fact("Running now", "Nothing"),
        ];
        assert_eq!(
            changes(&before, &after),
            ["Upscalers: None → FSR 2, built into the game"]
        );
        assert!(changes(&after, &after).is_empty());
        // A fact the first reading did not have is not a change.
        assert!(changes(&[], &after).is_empty());
    }
}
