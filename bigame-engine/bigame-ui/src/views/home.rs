//! The Home screen.
//!
//! One decision drives this view: a beginner should be able to open the
//! application, press one thing, and go and play. Turbo is that one thing —
//! the master switch. Off, BiGame-mode does not intervene in games; on, it
//! detects them and optimizes them.
//!
//! Everything fits one window without scrolling: three live readings, the
//! Turbo disc, the presets, and one card that says what Turbo did and, while
//! a game runs, which one and what it really got (one coloured flag per
//! feature). The full report is one button away, inside that card.
//!
//! Transitions run on a worker thread with its own Tokio runtime and report
//! back through a channel the GTK main loop drains, so D-Bus round trips and
//! Polkit prompts never stall the window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::hardware::Hardware;
use bigame_core::overview::{AppliedProfile, Snapshot, State as Fact};
use bigame_core::running::GameIdentity;
use bigame_core::turbo::{self, Report, Section, Step};

use crate::i18n::{error_text, i18n, ni18n};
use crate::widgets::booster_button::{self, BoosterButton, State};
use crate::widgets::sparkline::{self, SparkHandle};
use crate::widgets::status::Chip;

/// What the worker thread sends back to the UI.
enum Event {
    /// A stage began.
    Step(Step),
    /// The transition finished.
    Done(Box<Report>),
    /// It could not start at all.
    Failed(String),
}

/// How often the live readings refresh with no game running.
const TILE_REFRESH: std::time::Duration = std::time::Duration::from_secs(2);

/// Refresh every this many ticks while a game runs: 10 s instead of 2 s.
/// The readings are for glancing at, and the game is what should get the CPU.
const IN_GAME_EVERY: u32 = 5;

/// Latency is a `ping` process: every this many ticks.
const PING_EVERY: u32 = 5;

/// Build the Home page.
///
/// `show_report` is how the page asks the window to open the report, so
/// navigation stays the window's concern.
#[must_use]
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn build(show_report: Rc<dyn Fn(&Report)>) -> gtk4::Widget {
    let button = BoosterButton::new();
    let last_report: Rc<RefCell<Option<Report>>> = Rc::new(RefCell::new(Report::load_last()));

    let status = gtk4::Label::new(Some(&i18n("Checking your system…")));
    status.add_css_class("dim-label");
    status.add_css_class("home-status");
    status.set_wrap(true);
    status.set_justify(gtk4::Justification::Center);

    // ── Live readings, above the disc ───────────────────────────────────
    let readings = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    readings.set_halign(gtk4::Align::Center);
    let cpu_tile = MiniTile::new(&i18n("CPU"), "cpu-symbolic");
    let gpu_tile = MiniTile::new(&i18n("GPU"), "video-display-symbolic");
    let net_tile = MiniTile::new(&i18n("Network"), "network-wireless-symbolic");
    readings.append(cpu_tile.widget());
    readings.append(gpu_tile.widget());
    readings.append(net_tile.widget());

    // ── The card: Turbo's result, or the running game ───────────────────
    let card = InfoCard::new(Rc::clone(&last_report), Rc::clone(&show_report));

    // The preset is chosen before Turbo is switched on, and locked while it
    // is on or switching.
    let presets = crate::widgets::turbo_presets::PresetPicker::new();
    {
        let presets = Rc::clone(&presets);
        button.connect_state_changed(move |state| {
            presets.set_locked(
                !matches!(state, State::Off | State::Error { .. }),
                state.is_on(),
            );
        });
    }

    // Steam keeps the environment it started with: a preset switched on
    // while it is open reaches its games only once it is opened again. One
    // compact row, so the page still fits its window while it is shown.
    let steam_notice = SteamRow::new();

    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    column.set_halign(gtk4::Align::Center);
    column.set_valign(gtk4::Align::Center);
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    column.set_margin_start(18);
    column.set_margin_end(18);
    column.append(&status);
    column.append(&readings);
    column.append(button.widget());
    column.append(button.caption());
    column.append(presets.widget());
    column.append(steam_notice.widget());
    column.append(card.widget());

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&column)
        .vexpand(true)
        .build();

    let turbo_on = Rc::new(Cell::new(false));

    // ── Initial state, from the systems that hold it ────────────────────
    // The button is deliberately NOT focused on start-up: a focused button is
    // the target of any activation the toolkit delivers, and this one changes
    // system state. It is one Tab away.
    button.set_state(&State::Working {
        step: i18n("Reading Turbo's state"),
    });
    {
        let button = Rc::clone(&button);
        let turbo_on = Rc::clone(&turbo_on);
        let card = card.clone();
        glib::spawn_future_local(async move {
            let state = gtk4::gio::spawn_blocking(turbo::state_blocking).await;
            let on = matches!(state, Ok(Ok(turbo::State::On)));
            let readable = matches!(state, Ok(Ok(_)));
            turbo_on.set(on);
            card.turbo_readable.set(readable);
            // A preset lives only in the running session: after a login it
            // is set again while Turbo is on, and dropped if Turbo is off.
            if readable {
                gio::spawn_blocking(move || {
                    if let Err(e) = bigame_core::turbo_preset::resync(on) {
                        tracing::warn!(error = %format!("{e:#}"), "could not bring the Turbo preset back");
                    }
                });
            }
            let state = if !readable {
                // No system bus: saying "off" would be a guess.
                State::Error {
                    detail: i18n("Turbo's state cannot be read: systemd did not answer"),
                }
            } else if on {
                State::On {
                    detail: on_detail(crate::game_watch::current().as_ref()),
                }
            } else {
                State::Off
            };
            button.set_state(&state);
            booster_button::set_pulse(button.widget(), !on);
            card.show(crate::game_watch::current().as_ref(), on);
        });
    }

    // ── The running game ────────────────────────────────────────────────
    {
        let card = card.clone();
        let button = Rc::clone(&button);
        let turbo_on = Rc::clone(&turbo_on);
        crate::game_watch::subscribe(move |current| {
            card.show(current, turbo_on.get());
            if turbo_on.get() && matches!(button.state(), State::On { .. }) {
                button.set_state(&State::On {
                    detail: on_detail(current),
                });
            }
            glib::ControlFlow::Continue
        });
    }

    // ── Activation ──────────────────────────────────────────────────────
    {
        let button = Rc::clone(&button);
        let status = status.clone();
        let last = Rc::clone(&last_report);
        let show = Rc::clone(&show_report);
        let turbo_on = Rc::clone(&turbo_on);
        let card = card.clone();
        let steam_notice = steam_notice.clone();
        button.clone().connect_activated(move || {
            let turning_off = button.state().is_on();
            let working = if turning_off {
                State::Restoring
            } else {
                State::Working {
                    step: i18n("Reading hardware and tools"),
                }
            };
            button.set_state(&working);
            booster_button::set_pulse(button.widget(), false);

            let (tx, rx) = mpsc::channel::<Event>();
            spawn_worker(tx, turning_off);

            let button = Rc::clone(&button);
            let status = status.clone();
            let last = Rc::clone(&last);
            let show = Rc::clone(&show);
            let turbo_on = Rc::clone(&turbo_on);
            let card = card.clone();
            let steam_notice = steam_notice.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                loop {
                    let event = match rx.try_recv() {
                        Ok(event) => event,
                        Err(mpsc::TryRecvError::Empty) => break,
                        // The worker ended without an answer (it panicked):
                        // say so rather than stay on "working" for ever.
                        Err(mpsc::TryRecvError::Disconnected) => {
                            button.set_state(&State::Error {
                                detail: i18n(
                                    "The change stopped without an answer. Open Logs to see why.",
                                ),
                            });
                            return glib::ControlFlow::Break;
                        }
                    };
                    match event {
                        Event::Step(step) => {
                            if !turning_off {
                                button.set_state(&State::Working {
                                    step: step_text(&step),
                                });
                                button.set_progress(step_progress(&step));
                            }
                        }
                        Event::Done(report) => {
                            let report = *report;
                            let (state, on) = finished_state(&report);
                            steam_notice.set_visible(
                                on && bigame_core::turbo_preset::active()
                                    != bigame_core::turbo_preset::Preset::Standard
                                    && bigame_core::steam::is_running(),
                            );
                            turbo_on.set(on);
                            button.set_state(&state);
                            booster_button::set_pulse(button.widget(), !on);
                            status.set_label(&if on {
                                i18n("Games are optimized as they start")
                            } else {
                                i18n("BiGame-mode is not intervening in games")
                            });
                            let failed = report.count(Section::Failed) > 0;
                            *last.borrow_mut() = Some(report);
                            card.show(crate::game_watch::current().as_ref(), on);
                            crate::game_watch::check();
                            // Open the report on its own only when something
                            // went wrong; a clean run is summarised on Home.
                            if failed {
                                if let Some(r) = last.borrow().as_ref() {
                                    show(r);
                                }
                            }
                            return glib::ControlFlow::Break;
                        }
                        Event::Failed(detail) => {
                            button.set_state(&State::Error { detail });
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        });
    }

    // ── Turbo changed elsewhere ─────────────────────────────────────────
    // The command-line tool, systemctl, or another session can turn falcond
    // on or off; Home follows what systemd says rather than what it last did
    // itself. One D-Bus read every 10 s, only while the page is on screen.
    {
        let button = Rc::clone(&button);
        let turbo_on = Rc::clone(&turbo_on);
        let last = Rc::clone(&last_report);
        let card = card.clone();
        let root = scroll.clone();
        let busy = Rc::new(Cell::new(false));
        glib::timeout_add_local(std::time::Duration::from_secs(10), move || {
            if !root.is_mapped() || !button.state().is_interactive() || busy.replace(true) {
                return glib::ControlFlow::Continue;
            }
            let (button, turbo_on, last, card, busy) = (
                Rc::clone(&button),
                Rc::clone(&turbo_on),
                Rc::clone(&last),
                card.clone(),
                Rc::clone(&busy),
            );
            glib::spawn_future_local(async move {
                let reading = gio::spawn_blocking(|| {
                    (
                        bigame_core::systemd::Reader::shared()
                            .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT)),
                        Report::load_last(),
                    )
                })
                .await;
                busy.set(false);
                let Ok((Some(unit), report)) = reading else {
                    return;
                };
                let on = if unit.is_installed() {
                    unit.is_active()
                } else {
                    turbo_on.get()
                };
                if on != turbo_on.get()
                    || report.as_ref().map(|r| r.at) != last.borrow().as_ref().map(|r| r.at)
                {
                    turbo_on.set(on);
                    *last.borrow_mut() = report;
                    button.set_state(&if on {
                        State::On {
                            detail: on_detail(crate::game_watch::current().as_ref()),
                        }
                    } else {
                        State::Off
                    });
                    booster_button::set_pulse(button.widget(), !on);
                    card.show(crate::game_watch::current().as_ref(), on);
                }
            });
            glib::ControlFlow::Continue
        });
    }

    // ── Live readings ───────────────────────────────────────────────────
    {
        let status = status.clone();
        let card = card.clone();
        let root = scroll.clone();
        // Probed once: the CPU model and render GPU do not change while the
        // application runs, and re-probing every tick would be a full
        // hardware scan to update three numbers.
        let hw = Rc::new(Hardware::detect());
        status.set_label(&summary_line_machine(&hw));
        let tick = Cell::new(0u32);
        let net = net_tile.clone();
        let refresh = Refresh {
            update: Box::new(move |n| {
                if let Some(khz) = crate::views::details::telemetry::read_cpu_khz() {
                    #[allow(clippy::cast_precision_loss)]
                    let ghz = khz as f64 / 1_000_000.0;
                    cpu_tile.set(&format!("{ghz:.1} GHz"), ghz);
                }
                if let Some((text, value)) = gpu_reading(&hw) {
                    gpu_tile.set(&text, value);
                }
                if n % PING_EVERY == 1 {
                    let net = net.clone();
                    glib::spawn_future_local(async move {
                        let target = crate::settings::load().ping_target;
                        let ms = gio::spawn_blocking(move || {
                            crate::views::details::telemetry::read_ping_ms(&target)
                        })
                        .await
                        .ok()
                        .flatten();
                        match ms.and_then(|m| m.parse::<f64>().ok()) {
                            Some(ms) => net.set(&format!("{ms:.1} ms"), ms),
                            None => net.set(&net_fallback(), 0.0),
                        }
                    });
                }
                card.tick();
            }),
            root,
            tick,
        };
        // Once as soon as the page is shown, then on the timer -- otherwise
        // the tiles read "—" until the first tick, ten seconds into a game.
        let refresh = Rc::new(refresh);
        {
            let refresh = Rc::clone(&refresh);
            scroll.connect_map(move |_| {
                refresh.force();
            });
        }
        glib::timeout_add_local(TILE_REFRESH, move || refresh.tick());
    }

    scroll.upcast()
}

/// The live readings' refresh: forced when the page appears, then paced.
struct Refresh {
    /// Called with the tick number.
    update: Box<dyn Fn(u32)>,
    root: gtk4::ScrolledWindow,
    tick: Cell<u32>,
}

impl Refresh {
    fn force(&self) {
        (self.update)(1);
    }

    fn tick(&self) -> glib::ControlFlow {
        // Nothing to do while the window is hidden or on another page.
        if !self.root.is_mapped() {
            return glib::ControlFlow::Continue;
        }
        let n = self.tick.get().wrapping_add(1);
        self.tick.set(n);
        let playing = crate::game_watch::current().is_some();
        if !playing || n % IN_GAME_EVERY == 0 {
            (self.update)(n);
        }
        glib::ControlFlow::Continue
    }
}

/// The button's line while Turbo is on (its accessible description).
fn on_detail(game: Option<&GameIdentity>) -> String {
    match game {
        Some(g) => i18n("Optimizing %s").replace("%s", &g.display_name),
        None => i18n("Watching for games"),
    }
}

fn step_text(step: &Step) -> String {
    match step {
        Step::Detecting => i18n("Reading hardware and tools"),
        Step::ConfiguringProfiles => i18n("Choosing falcond's profile set"),
        Step::SwitchingBackend => i18n("Starting per-game optimization"),
        Step::Booster(_) => i18n("Checking global settings"),
        Step::Restoring => i18n("Putting global settings back"),
    }
}

/// How far switching on has got at `step`, for the artwork.
fn step_progress(step: &Step) -> f64 {
    match step {
        Step::Detecting => 0.1,
        Step::ConfiguringProfiles => 0.3,
        Step::SwitchingBackend => 0.5,
        Step::Booster(_) => 0.75,
        Step::Restoring => 0.0,
    }
}

/// The button's state, and whether Turbo is on, after a transition.
fn finished_state(report: &Report) -> (State, bool) {
    let failed = report.count(Section::Failed);
    let backend_failed = report
        .items
        .iter()
        .any(|i| i.section == Section::Failed && i.kind == turbo::Kind::GameBackend);
    if !report.turned_on {
        return if failed == 0 {
            (State::Off, false)
        } else {
            (
                State::Error {
                    detail: i18n("Some settings could not be put back"),
                },
                false,
            )
        };
    }
    if backend_failed {
        return (
            State::Error {
                detail: i18n("Per-game optimization could not be started"),
            },
            false,
        );
    }
    let state = if failed > 0 {
        State::Partial {
            detail: ni18n(
                "%n did not take effect — see details",
                "%n did not take effect — see details",
                failed,
            ),
        }
    } else {
        State::On {
            detail: on_detail(crate::game_watch::current().as_ref()),
        }
    };
    (state, true)
}

/// "2 applied · 3 per game · 1 skipped · 1 conflict avoided"
fn summary_line(report: &Report) -> String {
    let count = |section| report.count(section);
    [
        (
            Section::Verified,
            ni18n("%n applied", "%n applied", count(Section::Verified)),
        ),
        (
            Section::Restored,
            ni18n("%n restored", "%n restored", count(Section::Restored)),
        ),
        (
            Section::ManagedPerGame,
            ni18n("%n per game", "%n per game", count(Section::ManagedPerGame)),
        ),
        (
            Section::Skipped,
            ni18n("%n skipped", "%n skipped", count(Section::Skipped)),
        ),
        (
            Section::ConflictAvoided,
            ni18n(
                "%n conflict avoided",
                "%n conflicts avoided",
                count(Section::ConflictAvoided),
            ),
        ),
        (
            Section::Failed,
            ni18n("%n failed", "%n failed", count(Section::Failed)),
        ),
    ]
    .into_iter()
    .filter(|(section, _)| count(*section) > 0)
    .map(|(_, text)| text)
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Run a transition off the main thread.
fn spawn_worker(tx: mpsc::Sender<Event>, turning_off: bool) {
    let spawned = std::thread::Builder::new()
        .name("bigame-turbo".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Event::Failed(format!("{e}")));
                    return;
                }
            };
            runtime.block_on(async {
                let steps = tx.clone();
                let on_step = move |s| {
                    let _ = steps.send(Event::Step(s));
                };
                let result = if turning_off {
                    turbo::turn_off(on_step).await
                } else {
                    turbo::turn_on(on_step).await
                };
                let _ = tx.send(match result {
                    Ok(report) => Event::Done(Box::new(report)),
                    Err(e) => Event::Failed(error_text(&e)),
                });
            });
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the Turbo worker thread: {e}");
    }
}

// ── Steam, open before the preset ────────────────────────────────────────────

/// One row: Steam was open before the preset, and a button to open it again
/// in the session as it is now.
#[derive(Clone)]
struct SteamRow {
    root: gtk4::Box,
}

impl SteamRow {
    fn new() -> Self {
        let icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
        icon.add_css_class("accent");
        let title = gtk4::Label::new(Some(&i18n("Steam was already open")));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        let body = i18n(
            "It keeps the environment it started with, so its games get the preset once Steam is opened again. Opening it again closes it the way it closes itself.",
        );
        let text = gtk4::Label::new(Some(&body));
        text.add_css_class("caption");
        text.add_css_class("dim-label");
        text.set_xalign(0.0);
        text.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        text.set_tooltip_text(Some(&body));
        let lines = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        lines.set_valign(gtk4::Align::Center);
        lines.append(&title);
        lines.append(&text);
        let button = gtk4::Button::builder()
            .label(i18n("Reopen Steam"))
            .css_classes(["suggested-action", "pill"])
            .valign(gtk4::Align::Center)
            .build();
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        root.add_css_class("card");
        root.add_css_class("home-steam");
        root.set_visible(false);
        root.append(&icon);
        root.append(&lines);
        root.append(&button);
        let me = Self { root };
        {
            let me = me.clone();
            button.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (me, b) = (me.clone(), b.clone());
                glib::spawn_future_local(async move {
                    let done = gio::spawn_blocking(bigame_core::steam::restart_in_session).await;
                    b.set_sensitive(true);
                    match done {
                        Ok(Ok(())) => {
                            me.set_visible(false);
                            crate::widgets::toast::show(
                                &me.root,
                                &i18n("Steam opened again: its games get the preset"),
                            );
                        }
                        Ok(Err(e)) => crate::widgets::toast::error(
                            &me.root,
                            &i18n("Steam could not be opened again"),
                            &format!("{e:#}"),
                        ),
                        Err(_) => {}
                    }
                });
            });
        }
        me
    }

    fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }
}

// ── The card ─────────────────────────────────────────────────────────────────

/// One flag on the card: a feature, coloured by what it really got.
struct Flag {
    state: Fact,
    text: String,
}

impl Flag {
    fn new(state: Fact, text: impl Into<String>) -> Self {
        Self {
            state,
            text: text.into(),
        }
    }
}

/// What Turbo did, or the running game: cover, name, how it runs, and one
/// flag per feature, with the report's summary and its button.
#[derive(Clone)]
struct InfoCard {
    root: gtk4::Box,
    cover: gtk4::Image,
    icon: gtk4::Image,
    name: gtk4::Label,
    facts: gtk4::Label,
    flags: adw::WrapBox,
    summary: gtk4::Label,
    details: gtk4::Button,
    create: gtk4::Button,
    game: Rc<RefCell<Option<GameIdentity>>>,
    turbo_on: Rc<Cell<bool>>,
    /// Turbo's state could be read; when not, the card says nothing from it.
    turbo_readable: Rc<Cell<bool>>,
    /// The pid whose FSR 4 question was answered, and the answer
    /// ([`bigame_core::graphics::native_fsr4_applies`]): asked once per game.
    fsr4_applies: Rc<Cell<Option<(u32, bool)>>>,
    last_report: Rc<RefCell<Option<Report>>>,
}

impl InfoCard {
    fn new(last_report: Rc<RefCell<Option<Report>>>, show_report: Rc<dyn Fn(&Report)>) -> Self {
        // A fixed-size image, not a Picture: a Picture asks for the art's
        // natural size (600x900 for Steam's covers) and stretches the card.
        let cover = gtk4::Image::new();
        cover.set_pixel_size(88);
        cover.set_valign(gtk4::Align::Center);
        cover.add_css_class("home-cover");
        // With no game: Turbo's own symbol in the cover's place.
        let icon = gtk4::Image::from_icon_name("power-profile-performance-symbolic");
        icon.set_pixel_size(36);
        icon.set_valign(gtk4::Align::Center);
        icon.add_css_class("home-card-icon");

        let name = gtk4::Label::new(None);
        name.add_css_class("title-4");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name.set_max_width_chars(40);
        let facts = gtk4::Label::new(None);
        facts.add_css_class("caption");
        facts.add_css_class("dim-label");
        facts.set_xalign(0.0);
        facts.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        facts.set_max_width_chars(60);

        let flags = adw::WrapBox::new();
        flags.set_child_spacing(6);
        flags.set_line_spacing(6);

        let summary = gtk4::Label::new(None);
        summary.add_css_class("caption");
        summary.add_css_class("dim-label");
        summary.set_xalign(0.0);
        summary.set_hexpand(true);
        summary.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        let details = gtk4::Button::builder()
            .label(i18n("View optimization details"))
            .css_classes(["flat", "home-details"])
            .halign(gtk4::Align::End)
            .build();
        {
            let last = Rc::clone(&last_report);
            details.connect_clicked(move |_| {
                if let Some(report) = last.borrow().as_ref() {
                    show_report(report);
                }
            });
        }
        let create = gtk4::Button::builder()
            .label(i18n("Create profile"))
            .css_classes(["pill", "suggested-action"])
            .halign(gtk4::Align::End)
            .visible(false)
            .build();
        create.set_action_name(Some("app.profile-review"));
        // The action takes the process name. Until a game is running there is
        // none, but without a target of the right type GTK rejects the
        // button on every update ("parameter type mismatch").
        create.set_action_target_value(Some(&glib::variant::ToVariant::to_variant("")));
        let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        bottom.append(&summary);
        bottom.append(&create);
        bottom.append(&details);

        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        text.set_valign(gtk4::Align::Center);
        text.set_hexpand(true);
        text.append(&name);
        text.append(&facts);
        text.append(&flags);
        text.append(&bottom);

        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        root.add_css_class("card");
        root.add_css_class("home-card");
        root.set_halign(gtk4::Align::Fill);
        root.set_size_request(560, -1);
        root.append(&cover);
        root.append(&icon);
        root.append(&text);

        let me = Self {
            root,
            cover,
            icon,
            name,
            facts,
            flags,
            summary,
            details,
            create,
            game: Rc::new(RefCell::new(None)),
            turbo_on: Rc::new(Cell::new(false)),
            turbo_readable: Rc::new(Cell::new(true)),
            fsr4_applies: Rc::new(Cell::new(None)),
            last_report,
        };
        me.show(None, false);
        me
    }

    fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    fn set_flags(&self, flags: &[Flag]) {
        while let Some(child) = self.flags.first_child() {
            self.flags.remove(&child);
        }
        for f in flags {
            let chip = Chip::new(f.state);
            chip.set(f.state, Some(&f.text));
            self.flags.append(chip.widget());
        }
        self.flags.set_visible(!flags.is_empty());
    }

    /// The report's summary line and its button.
    fn show_report(&self) {
        let last = self.last_report.borrow();
        let text = last.as_ref().map(summary_line).unwrap_or_default();
        self.summary.set_label(&text);
        self.summary.set_visible(!text.is_empty());
        self.details.set_visible(last.is_some());
    }

    fn show(&self, game: Option<&GameIdentity>, turbo_on: bool) {
        *self.game.borrow_mut() = game.cloned();
        self.turbo_on.set(turbo_on);
        self.show_report();
        let Some(g) = game else {
            self.cover.set_visible(false);
            self.icon.set_visible(true);
            self.create.set_visible(false);
            self.facts.set_visible(true);
            if turbo_on {
                self.name.set_label(&i18n("Watching for games"));
                self.facts
                    .set_label(&i18n("The next game gets its profile as it starts."));
                let preset = bigame_core::turbo_preset::active();
                let mut flags = vec![Flag::new(Fact::Active, i18n("Turbo"))];
                if preset != bigame_core::turbo_preset::Preset::Standard {
                    flags.push(Flag::new(Fact::Active, i18n(preset.label())));
                }
                self.set_flags(&flags);
            } else {
                self.name.set_label(&i18n("Turbo is off"));
                self.facts
                    .set_label(&i18n("Games run without BiGame-mode's optimizations."));
                self.set_flags(&[Flag::new(Fact::Off, i18n("Turbo"))]);
            }
            self.root.set_visible(self.turbo_readable.get());
            return;
        };
        self.root.set_visible(true);
        self.icon.set_visible(false);
        self.name.set_label(&g.display_name);
        // Steam's cover by its id; any other launcher's from the library.
        // Found and decoded off the main thread at the size shown; dropped
        // if another game took the card meanwhile.
        self.cover.set_visible(false);
        {
            let image = self.cover.clone();
            let shown = Rc::clone(&self.game);
            let pid = g.pid;
            let size = image.pixel_size() * image.scale_factor().max(1);
            let (app_id, process, folder) = (
                g.steam_app_id.clone(),
                g.process_name.clone(),
                g.install_path.clone(),
            );
            let name = self.name.clone();
            let bare_name = g.display_name == g.process_name;
            glib::spawn_future_local(async move {
                let found = gio::spawn_blocking(move || {
                    let steam = app_id.as_ref().and_then(|id| {
                        let home = std::env::var_os("HOME")?;
                        bigame_core::games::steam_cover(std::path::Path::new(&home), id)
                    });
                    // Only a process name: the library knows the title.
                    let installed = (steam.is_none() || bare_name)
                        .then(|| {
                            bigame_core::games::installed_game_for_process(
                                &process,
                                folder.as_deref(),
                            )
                        })
                        .flatten();
                    let title = installed.as_ref().map(|g| g.name.clone());
                    let texture =
                        steam
                            .or_else(|| installed.and_then(|g| g.cover))
                            .and_then(|path| {
                                gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(
                                    &path, size, size, true,
                                )
                                .ok()
                                .map(|p| gtk4::gdk::Texture::for_pixbuf(&p))
                            });
                    (title, texture)
                })
                .await
                .ok();
                let still_shown = shown.borrow().as_ref().is_some_and(|g| g.pid == pid);
                let Some((title, texture)) = found.filter(|_| still_shown) else {
                    return;
                };
                if let Some(title) = title.filter(|_| bare_name) {
                    name.set_label(&title);
                }
                if let Some(texture) = texture {
                    image.set_paintable(Some(&texture));
                    image.set_visible(true);
                }
            });
        }
        self.create
            .set_action_target_value(Some(&glib::variant::ToVariant::to_variant(&g.process_name)));
        self.tick();
    }

    /// Refresh what changes while the game runs.
    fn tick(&self) {
        let Some(g) = self.game.borrow().clone() else {
            return;
        };
        let mut facts = vec![match &g.runtime {
            bigame_core::running::Runtime::Native => i18n("Native"),
            bigame_core::running::Runtime::Proton(tool) if !tool.is_empty() => tool.clone(),
            bigame_core::running::Runtime::Proton(_) => "Proton".into(),
            bigame_core::running::Runtime::Wine => "Wine".into(),
        }];
        if g.graphics != bigame_core::running::Graphics::Unknown {
            facts.push(g.graphics.label().to_owned());
        }
        if let Some(secs) = bigame_core::running::running_for(g.pid) {
            facts.push(format!(
                "{:02}:{:02}:{:02}",
                secs / 3600,
                (secs / 60) % 60,
                secs % 60
            ));
        }
        self.facts.set_label(&facts.join(" · "));
        // Everything the flags say is read from the running game and the
        // kernel, as Details reads it; the FSR 4 question is asked once per
        // game. All of it off the main thread.
        let me = self.clone();
        let fsr4_cache = Rc::clone(&self.fsr4_applies);
        let known = fsr4_cache
            .get()
            .filter(|(pid, _)| *pid == g.pid)
            .map(|(_, a)| a);
        let turbo_on = self.turbo_on.get();
        let turbo_readable = self.turbo_readable.get();
        glib::spawn_future_local(async move {
            let pid = g.pid;
            let Ok((snap, applies, native_fsr4)) = gio::spawn_blocking(move || {
                let applies =
                    known.unwrap_or_else(|| bigame_core::graphics::native_fsr4_applies(&g));
                let native = applies
                    .then(|| bigame_core::graphics::native_fsr4_loaded(g.pid))
                    .flatten();
                (Snapshot::collect(Some(g)), applies, native)
            })
            .await
            else {
                return;
            };
            if me.game.borrow().as_ref().map(|g| g.pid) != Some(pid) {
                return;
            }
            fsr4_cache.set(Some((pid, applies)));
            let flags = game_flags(&snap, native_fsr4, turbo_on && turbo_readable);
            me.set_flags(&flags);
            let offer = turbo_readable
                && turbo_on
                && matches!(
                    snap.profile,
                    AppliedProfile::None | AppliedProfile::GenericProton
                );
            me.create.set_visible(offer);
        });
    }
}

/// One flag per feature for the running game, from what Details reads.
fn game_flags(snap: &Snapshot, native_fsr4: Option<bool>, turbo: bool) -> Vec<Flag> {
    use crate::views::details::overview::{frame_generation_summary, upscaling_summary};
    let mut flags = Vec::new();
    if turbo {
        flags.push(match &snap.profile {
            AppliedProfile::Own { name, .. } | AppliedProfile::Other(name) => {
                Flag::new(Fact::Active, name.clone())
            }
            AppliedProfile::GenericProton => Flag::new(Fact::Active, i18n("Proton (general)")),
            AppliedProfile::None => Flag::new(Fact::NotDetected, i18n("No profile")),
        });
        let sched = snap.scheduler.loaded.clone().or_else(|| {
            (!snap.scheduler.requested.is_empty() && snap.scheduler.requested != "none")
                .then(|| snap.scheduler.requested.clone())
        });
        flags.push(Flag::new(
            snap.scheduler.state(true),
            sched.map_or_else(|| i18n("Scheduler"), |s| format!("scx_{s}")),
        ));
        if let Some(p) = &snap.power_profile {
            flags.push(Flag::new(snap.power_state(), p.clone()));
        }
    } else {
        flags.push(Flag::new(Fact::Off, i18n("Turbo")));
    }
    flags.push(Flag::new(snap.gamescope_state(), "Gamescope"));
    let (state, text) = upscaling_summary(snap);
    match (native_fsr4, state) {
        (Some(true), Fact::Off) => flags.push(Flag::new(Fact::Active, "FSR 4")),
        (_, state) => flags.push(Flag::new(state, text.unwrap_or_else(|| i18n("Upscaling")))),
    }
    let (state, text) = frame_generation_summary(snap);
    flags.push(Flag::new(
        state,
        text.unwrap_or_else(|| i18n("Frame generation")),
    ));
    flags.push(Flag::new(snap.mangohud_state(), "MangoHud"));
    flags.push(Flag::new(
        snap.vkbasalt_state(),
        if snap
            .in_game
            .as_ref()
            .is_some_and(|g| !g.vkbasalt && g.vkbasalt_in_gamescope)
        {
            "vkBasalt (Gamescope)"
        } else {
            "vkBasalt"
        },
    ));
    if let Some(fps) = snap.in_game.as_ref().and_then(|g| g.frame_cap) {
        flags.push(Flag::new(Fact::Active, format!("{fps} FPS")));
    }
    flags
}

// ── Readings ─────────────────────────────────────────────────────────────────

/// One-line description of the machine: the CPU and the GPU games use.
fn summary_line_machine(hw: &Hardware) -> String {
    let gpu = bigame_core::graphics::report::render_gpu(hw)
        .map(|g| bigame_core::graphics::report::display_name(&g.name))
        .filter(|n| !n.contains(':'))
        .or_else(|| hw.render_gpu().map(short_gpu))
        .map_or_else(String::new, |g| format!(" · {g}"));
    format!("{}{}", short_cpu(&hw.cpu.model), gpu)
}

/// Trim vendor boilerplate so the line stays readable at small widths.
pub(crate) fn short_cpu(model: &str) -> String {
    if model == bigame_core::hardware::UNKNOWN_CPU {
        return i18n(bigame_core::hardware::UNKNOWN_CPU);
    }
    model
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace(" with Radeon Graphics", "")
        .replace("CPU ", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn short_gpu(gpu: &bigame_core::hardware::Gpu) -> String {
    match gpu.vendor {
        bigame_core::hardware::GpuVendor::Amd => i18n("%s GPU").replace("%s", "AMD"),
        bigame_core::hardware::GpuVendor::Nvidia => i18n("%s GPU").replace("%s", "NVIDIA"),
        bigame_core::hardware::GpuVendor::Intel => i18n("%s GPU").replace("%s", "Intel"),
        bigame_core::hardware::GpuVendor::Other => gpu.driver.clone(),
    }
}

/// The card games use: the one the running game has open, else the expected
/// one. Its load when the driver reports it, else its temperature.
fn gpu_reading(hw: &Hardware) -> Option<(String, f64)> {
    let running = crate::game_watch::current().and_then(|g| g.render_card);
    let gpu = bigame_core::gpu_telemetry::games_gpu(&hw.gpus, running.as_deref())
        .and_then(|i| hw.gpus.get(i))?;
    let s = bigame_core::gpu_telemetry::sample(gpu);
    if s.asleep {
        return Some((i18n("Asleep"), 0.0));
    }
    match (s.busy_pct, s.temp_c) {
        (Some(b), _) => Some((format!("{b}%"), f64::from(b))),
        #[allow(clippy::cast_possible_truncation)]
        (None, Some(t)) => Some((format!("{} °C", t.round() as i64), t)),
        (None, None) => None,
    }
}

/// The network reading when there is no latency to show.
fn net_fallback() -> String {
    bigame_core::network::primary_link_brief().map_or_else(
        || i18n("Offline"),
        |l| match l.speed_mbps {
            Some(mbps) => format!("{mbps} Mb/s"),
            None => l.name,
        },
    )
}

/// A small live reading with its chart.
#[derive(Clone)]
struct MiniTile {
    root: gtk4::Box,
    value: gtk4::Label,
    spark: SparkHandle,
}

impl MiniTile {
    fn new(label: &str, icon: &str) -> Self {
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let image = gtk4::Image::from_icon_name(icon);
        image.set_pixel_size(14);
        image.add_css_class("dim-label");
        let caption = gtk4::Label::new(Some(label));
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        let value = gtk4::Label::new(Some("—"));
        value.add_css_class("heading");
        value.set_hexpand(true);
        value.set_xalign(1.0);
        header.append(&image);
        header.append(&caption);
        header.append(&value);

        let spark = sparkline::build();
        spark.area.set_content_height(18);
        spark.area.set_content_width(120);

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        root.add_css_class("card");
        root.add_css_class("home-mini");
        root.set_width_request(150);
        root.append(&header);
        root.append(&spark.area);
        root.update_property(&[gtk4::accessible::Property::Label(label)]);

        Self { root, value, spark }
    }

    fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    fn set(&self, text: &str, value: f64) {
        self.value.set_label(text);
        self.spark.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_model_is_trimmed_for_display() {
        assert_eq!(
            short_cpu("AMD Ryzen 7 5700G with Radeon Graphics"),
            "AMD Ryzen 7 5700G"
        );
        assert_eq!(
            short_cpu("Intel(R) Core(TM) i7-12700K CPU @ 3.60GHz"),
            "Intel Core i7-12700K @ 3.60GHz"
        );
        assert!(!short_cpu("Intel(R)  Core(TM)  i5").contains("  "));
    }

    fn report(sections: &[Section]) -> Report {
        Report {
            turned_on: true,
            at: 0,
            items: sections
                .iter()
                .map(|s| turbo::Item {
                    kind: turbo::Kind::Knob("x".into()),
                    section: *s,
                    owner: "falcond".into(),
                    detail: String::new(),
                    text: None,
                    title: None,
                })
                .collect(),
        }
    }

    #[test]
    fn the_summary_names_only_what_happened() {
        let r = report(&[
            Section::Verified,
            Section::ManagedPerGame,
            Section::ManagedPerGame,
            Section::Skipped,
        ]);
        assert_eq!(summary_line(&r), "1 applied · 2 per game · 1 skipped");
        assert_eq!(summary_line(&report(&[])), "");
    }

    #[test]
    fn a_backend_that_did_not_start_is_not_reported_as_on() {
        let mut r = report(&[]);
        r.items.push(turbo::Item {
            kind: turbo::Kind::GameBackend,
            section: Section::Failed,
            owner: "falcond".into(),
            detail: "systemd reports it failed".into(),
            text: None,
            title: None,
        });
        let (state, on) = finished_state(&r);
        assert!(!on);
        assert!(matches!(state, State::Error { .. }));
    }

    #[test]
    fn a_partial_failure_is_on_but_says_so() {
        let (state, on) = finished_state(&report(&[Section::Verified, Section::Failed]));
        assert!(on);
        assert!(matches!(state, State::Partial { .. }));
    }

    #[test]
    fn the_flags_say_what_the_game_got() {
        let snap = Snapshot::default();
        let flags = game_flags(&snap, None, true);
        let texts: Vec<&str> = flags.iter().map(|f| f.text.as_str()).collect();
        assert!(texts.contains(&"Gamescope") && texts.contains(&"MangoHud"));
        assert!(flags.iter().all(|f| !f.text.is_empty()));
        // FSR 4 loaded by the game's own path shows even with nothing else.
        let flags = game_flags(&snap, Some(true), false);
        assert!(
            flags
                .iter()
                .any(|f| f.text == "FSR 4" && f.state == Fact::Active)
        );
    }
}
