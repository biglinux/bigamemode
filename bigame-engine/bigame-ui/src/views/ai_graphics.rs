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

use bigame_core::graphics::config::{AiGraphicsConfig, FrameGeneration, Layer, Mode, Upscaler};
use bigame_core::graphics::plan::{Standing, Step};
use bigame_core::graphics::report::Confidence;
use bigame_core::graphics::runtime::Status;
use bigame_core::graphics::{self, Analysis, Target};

use crate::i18n::{i18n, ni18n};

struct Page {
    target: Target,
    cfg: RefCell<AiGraphicsConfig>,
    analysis: RefCell<Option<Analysis>>,
    body: gtk4::Box,
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
        Status::Active { upscaler, .. } => {
            format!("{} ({})", i18n("Active"), upscaler_name(upscaler))
        }
        Status::NotDetected => i18n("Installed, but the game did not load it"),
        Status::Failed { errors } => format!(
            "{}: {}",
            i18n("Failed"),
            errors.first().cloned().unwrap_or_default()
        ),
    }
}

/// `OptiScaler` backend ids, as people know them. FSR 4 is never claimed
/// from the backend alone: `fsr31` runs FSR 4 only where the GPU and the
/// runtime allow it, which the log does not say.
fn upscaler_name(backend: &str) -> String {
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

/// A sentence from bigame-core, translated: the template through gettext,
/// then its values filled in.
fn tr(t: &bigame_core::graphics::text::Text) -> String {
    bigame_core::graphics::text::Text::fill(&i18n(t.template), &t.args)
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

    // ── Now ──────────────────────────────────────────────────────────
    let now = adw::PreferencesGroup::new();
    now.set_title(&i18n("Now"));
    now.add(&row(&i18n("Status"), &status_text(&a.status)));
    page.body.append(&now);

    // ── Recommendation ───────────────────────────────────────────────
    let rec = adw::PreferencesGroup::new();
    rec.set_title(&sentence(&tr(&p.summary)));
    rec.set_description(Some(&if r.installed.is_some() {
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
             graphics cards. Frame generation is never switched on by itself: it raises the \
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

    // ── Choose yourself ──────────────────────────────────────────────
    page.body.append(&advanced_group(page));

    page.body.append(&found_group(r));

    // ── Buttons ──────────────────────────────────────────────────────
    let installed = r.installed.is_some();
    page.apply.set_visible(!installed && p.optiscaler.is_some());
    page.repair.set_visible(installed);
    page.remove.set_visible(installed);
}

/// "What was found": the evidence behind the plan, for whoever wants it.
fn found_group(r: &bigame_core::graphics::report::Report) -> adw::PreferencesGroup {
    let found = adw::PreferencesGroup::new();
    let details = adw::ExpanderRow::builder()
        .title(i18n("What was found"))
        .subtitle(i18n(
            "Graphics API, GPU, the game's own upscalers, DLL slots",
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
        if let Ok(a) = analysis {
            render(&page, &a);
            *page.analysis.borrow_mut() = Some(a);
        }
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
    let mut cfg = bigame_core::game_settings::load(&target.process)
        .map(|s| s.ai_graphics)
        .unwrap_or_default();
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
                busy(&page, Some(&i18n("Downloading, checking and installing…")));
                save_settings(&page);
                let target = page.target.clone();
                let cfg = page.cfg.borrow().clone();
                let result = gio::spawn_blocking(move || {
                    let a = graphics::analyze(&target, &cfg);
                    graphics::install(&target, &a.plan)
                })
                .await;
                busy(&page, None);
                let text = match result {
                    Ok(Ok(m)) => format!(
                        "{} ({})",
                        i18n("Installed; every replaced file was backed up"),
                        ni18n("%n file", "%n files", m.entries.len())
                    ),
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
