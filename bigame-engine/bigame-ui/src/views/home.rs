//! The Home screen.
//!
//! One decision drives this view: a beginner should be able to open the
//! application, press one thing, and go and play. Turbo is that one thing —
//! the master switch. Off, Big Game Mode does not intervene in games; on, it
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

use bigame_core::turbo_preset;

use crate::i18n::{error_text, i18n, ni18n, tr};
use crate::widgets::booster_button::{self, BoosterButton, State};
use crate::widgets::launcher_notice::LauncherNotice;
use crate::widgets::sparkline::{self, SparkHandle};
use crate::widgets::status::Chip;
use crate::widgets::turbo_presets::{Mode, PresetPicker};

/// What the worker thread sends back to the UI.
enum Event {
    /// A stage began.
    Step(Step),
    /// The transition finished, and whether Turbo is on afterwards as
    /// systemd says (`None` when it could not be read).
    Done(Box<Report>, Option<bool>),
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
/// navigation stays the window's concern. Turbo and its preset are given to
/// `app` as the stateful actions `app.turbo` and `app.turbo-preset`: their
/// state is what this page shows, read from the systems that hold it, and
/// changing it runs what the Turbo button and the preset picker run. The
/// tray reads and changes Turbo through them, never beside them.
#[must_use]
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn build(
    app: &adw::Application,
    show_report: Rc<dyn Fn(&Report)>,
    launcher_notice: Rc<LauncherNotice>,
) -> gtk4::Widget {
    let button = BoosterButton::new();
    let last_report: Rc<RefCell<Option<Report>>> = Rc::new(RefCell::new(Report::load_last()));

    // The machine, named once its hardware has been read.
    let machine = gtk4::Label::new(Some(&i18n("Checking your system…")));
    machine.add_css_class("dim-label");
    machine.add_css_class("home-status");
    machine.set_wrap(true);
    machine.set_justify(gtk4::Justification::Center);

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

    let presets = PresetPicker::new();

    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    column.set_halign(gtk4::Align::Center);
    column.set_valign(gtk4::Align::Center);
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    column.set_margin_start(18);
    column.set_margin_end(18);
    column.append(&machine);
    column.append(&readings);
    column.append(button.widget());
    column.append(button.caption());
    column.append(presets.widget());
    column.append(card.widget());

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&column)
        .vexpand(true)
        .build();

    let turbo_on = Rc::new(Cell::new(false));
    // A preset is being put in force while Turbo is on: Turbo and the
    // presets wait for it.
    let switching_preset = Rc::new(Cell::new(false));

    // ── Turbo and its preset, for the application (the tray) ────────────
    let turbo_action = gio::SimpleAction::new_stateful("turbo", None, &false.to_variant());
    turbo_action.set_enabled(false);
    let preset_action = gio::SimpleAction::new_stateful(
        "turbo-preset",
        Some(glib::VariantTy::STRING),
        &presets.shown().id().to_variant(),
    );
    preset_action.set_enabled(false);
    app.add_action(&turbo_action);
    app.add_action(&preset_action);
    // What the page shows, handed to the actions: called whenever it changes.
    let publish: Rc<dyn Fn()> = {
        let (button, presets, switching) = (
            Rc::clone(&button),
            Rc::clone(&presets),
            Rc::clone(&switching_preset),
        );
        let (turbo_action, preset_action) = (turbo_action.clone(), preset_action.clone());
        Rc::new(move || {
            let state = button.state();
            turbo_action.set_state(&state.is_on().to_variant());
            turbo_action.set_enabled(state.is_interactive() && !switching.get());
            preset_action.set_state(&presets.shown().id().to_variant());
            preset_action.set_enabled(presets.is_open());
        })
    };

    // With Turbo off a pick is for when it is switched on; with Turbo on it
    // changes the preset in force. Nothing is picked while either switches.
    {
        let (presets, switching, publish) = (
            Rc::clone(&presets),
            Rc::clone(&switching_preset),
            Rc::clone(&publish),
        );
        button.connect_state_changed(move |state| {
            let mode = if !state.is_interactive() || switching.get() {
                Mode::Locked
            } else if state.is_on() {
                // The one in force is the one shown.
                presets.show(turbo_preset::active());
                Mode::Live
            } else {
                Mode::Next
            };
            presets.set_mode(mode);
            publish();
        });
    }
    {
        let presets_weak = Rc::downgrade(&presets);
        let (button, switching, publish, card, turbo_on, notice) = (
            Rc::clone(&button),
            Rc::clone(&switching_preset),
            Rc::clone(&publish),
            card.clone(),
            Rc::clone(&turbo_on),
            Rc::clone(&launcher_notice),
        );
        presets.connect_picked(move |preset| {
            let Some(presets) = presets_weak.upgrade() else {
                return;
            };
            if !button.state().is_on() {
                if let Err(e) = turbo_preset::set_chosen(preset) {
                    crate::widgets::toast::error(
                        presets.widget(),
                        &i18n("The preset could not be changed"),
                        &error_text(&e),
                    );
                }
                publish();
                return;
            }
            // Turbo is on: the session's environment changes, off the main
            // thread, and nothing else switches meanwhile.
            switching.set(true);
            presets.set_mode(Mode::Locked);
            button.widget().set_sensitive(false);
            publish();
            let (button, switching, publish, card, turbo_on, notice) = (
                Rc::clone(&button),
                Rc::clone(&switching),
                Rc::clone(&publish),
                card.clone(),
                Rc::clone(&turbo_on),
                Rc::clone(&notice),
            );
            // Quit waits for the session's environment to be in one state.
            let busy = crate::app::Busy::hold();
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || turbo_preset::switch(preset)).await;
                drop(busy);
                switching.set(false);
                let state = button.state();
                button.widget().set_sensitive(state.is_interactive());
                match result {
                    Ok(Ok(_)) => {
                        tracing::info!(target: "turbo", preset = preset.id(), "Turbo preset changed");
                        notice.check(true);
                    }
                    Ok(Err(e)) => crate::widgets::toast::error(
                        presets.widget(),
                        &i18n("The preset could not be changed"),
                        &error_text(&e),
                    ),
                    Err(_) => crate::widgets::toast::error(
                        presets.widget(),
                        &i18n("The preset could not be changed"),
                        &i18n("the worker thread stopped"),
                    ),
                }
                // What is in force, read back.
                presets.show(turbo_preset::active());
                presets.set_mode(if !state.is_interactive() {
                    Mode::Locked
                } else if state.is_on() {
                    Mode::Live
                } else {
                    Mode::Next
                });
                card.show(crate::game_watch::current().as_ref(), turbo_on.get());
                card.refresh_preset();
                publish();
            });
        });
    }
    // The tray asks for a preset as a click on it would.
    {
        let presets = Rc::downgrade(&presets);
        preset_action.connect_change_state(move |_, value| {
            let Some(preset) = value
                .and_then(glib::Variant::str)
                .and_then(turbo_preset::Preset::from_id)
            else {
                return;
            };
            if let Some(presets) = presets.upgrade()
                && presets.is_open()
                && presets.shown() != preset
            {
                presets.select(preset);
            }
        });
    }

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
        let notice = Rc::clone(&launcher_notice);
        glib::spawn_future_local(async move {
            let state = gtk4::gio::spawn_blocking(turbo::state_blocking).await;
            let on = matches!(state, Ok(Ok(turbo::State::On)));
            let readable = matches!(state, Ok(Ok(_)));
            turbo_on.set(on);
            card.turbo_readable.set(readable);
            // A preset lives only in the running session: after a login it
            // is set again while Turbo is on, and dropped if Turbo is off.
            // Then a launcher opened before that is asked to reopen.
            if readable {
                let card = card.clone();
                glib::spawn_future_local(async move {
                    let _ = gio::spawn_blocking(move || {
                        if let Err(e) = turbo_preset::resync(on) {
                            tracing::warn!(error = %format!("{e:#}"), "could not bring the Turbo preset back");
                        }
                    })
                    .await;
                    card.refresh_preset();
                    if on {
                        notice.check(false);
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

    // ── Activation: the button, or `app.turbo` from the tray ────────────
    let start: Rc<dyn Fn(bool)> = {
        let button = Rc::clone(&button);
        let last = Rc::clone(&last_report);
        let show = Rc::clone(&show_report);
        let turbo_on = Rc::clone(&turbo_on);
        let card = card.clone();
        let notice = Rc::clone(&launcher_notice);
        let switching = Rc::clone(&switching_preset);
        Rc::new(move |turning_off| {
            if !button.state().is_interactive() || switching.get() {
                return;
            }
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
            // Quit waits for the transition: half of Turbo on (or off) is a
            // state no surface describes. Released when the result is in.
            let busy = std::cell::RefCell::new(Some(crate::app::Busy::hold()));

            let button = Rc::clone(&button);
            let last = Rc::clone(&last);
            let show = Rc::clone(&show);
            let turbo_on = Rc::clone(&turbo_on);
            let card = card.clone();
            let notice = Rc::clone(&notice);
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                loop {
                    let event = match rx.try_recv() {
                        Ok(event) => event,
                        Err(mpsc::TryRecvError::Empty) => break,
                        // The worker ended without an answer (it panicked):
                        // say so rather than stay on "working" for ever.
                        Err(mpsc::TryRecvError::Disconnected) => {
                            busy.borrow_mut().take();
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
                        Event::Done(report, now) => {
                            busy.borrow_mut().take();
                            let report = *report;
                            let (state, on) = finished_state(&report, now);
                            turbo_on.set(on);
                            button.set_state(&state);
                            booster_button::set_pulse(button.widget(), !on);
                            // The session's environment changed: a launcher
                            // opened before keeps the old one.
                            notice.check(true);
                            let failed = report.count(Section::Failed) > 0;
                            *last.borrow_mut() = Some(report);
                            card.show(crate::game_watch::current().as_ref(), on);
                            card.refresh_preset();
                            crate::game_watch::check();
                            // Open the report on its own only when something
                            // went wrong; a clean run is summarised on Home.
                            if failed && let Some(r) = last.borrow().as_ref() {
                                show(r);
                            }
                            return glib::ControlFlow::Break;
                        }
                        Event::Failed(detail) => {
                            busy.borrow_mut().take();
                            button.set_state(&State::Error { detail });
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        })
    };
    // On switches off, off switches on. After an error, what a failed start
    // may have left (falcond enabled, a preset, Booster changes) is switched
    // off first: an error must never be a state nothing can leave.
    let toggle: Rc<dyn Fn()> = {
        let button = Rc::clone(&button);
        let switching = Rc::clone(&switching_preset);
        Rc::new(move || {
            let state = button.state();
            if !state.is_interactive() || switching.get() {
                return;
            }
            if !matches!(state, State::Error { .. }) {
                start(state.is_on());
                return;
            }
            let start = Rc::clone(&start);
            glib::spawn_future_local(async move {
                let left = gio::spawn_blocking(turbo::something_left_blocking)
                    .await
                    .unwrap_or(true);
                start(left);
            });
        })
    };
    {
        let toggle = Rc::clone(&toggle);
        button.connect_activated(move || toggle());
    }
    {
        let button = Rc::clone(&button);
        turbo_action.connect_change_state(move |_, value| {
            // A request, not the result: the state follows once Turbo has
            // really switched.
            if let Some(wanted) = value.and_then(bool::from_variant)
                && wanted != button.state().is_on()
            {
                toggle();
            }
        });
    }

    // ── Turbo changed elsewhere ─────────────────────────────────────────
    // The command-line tool, systemctl, or another session can turn falcond
    // on or off; Home follows what systemd says rather than what it last did
    // itself. One D-Bus read every 10 s, with the window hidden too, so the
    // tray never shows a Turbo that is no longer so.
    {
        let button = Rc::clone(&button);
        let turbo_on = Rc::clone(&turbo_on);
        let last = Rc::clone(&last_report);
        let card = card.clone();
        let notice = Rc::clone(&launcher_notice);
        let switching = Rc::clone(&switching_preset);
        let root = scroll.clone();
        let busy = Rc::new(Cell::new(false));
        glib::timeout_add_local(std::time::Duration::from_secs(10), move || {
            if !button.state().is_interactive() || switching.get() || busy.replace(true) {
                return glib::ControlFlow::Continue;
            }
            let (button, turbo_on, last, card, busy, notice, root) = (
                Rc::clone(&button),
                Rc::clone(&turbo_on),
                Rc::clone(&last),
                card.clone(),
                Rc::clone(&busy),
                Rc::clone(&notice),
                root.clone(),
            );
            glib::spawn_future_local(async move {
                let reading = gio::spawn_blocking(|| {
                    // A falcond that crashed on this processor is not
                    // Turbo's state: Turbo is the Booster's then, as without
                    // falcond, and its unit staying inactive is not Turbo
                    // going off.
                    let governing = bigame_core::systemd::Reader::shared()
                        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
                        .map(|unit| turbo::backend_governs(&unit).then(|| unit.is_active()));
                    (governing, Report::load_last())
                })
                .await;
                busy.set(false);
                let Ok((Some(governing), report)) = reading else {
                    return;
                };
                let on = governing.unwrap_or_else(|| turbo_on.get());
                // A launcher opened again, or closed, by hand leaves the
                // notice.
                if root.is_mapped() {
                    notice.recheck();
                }
                // The preset's flag follows what the session holds.
                card.refresh_preset();
                if on != turbo_on.get()
                    || report.as_ref().map(|r| r.at) != last.borrow().as_ref().map(|r| r.at)
                {
                    let switched = on != turbo_on.get();
                    // falcond stopped without Big Game Mode (systemctl, a
                    // crash, Settings → Hand back): what Turbo laid over the
                    // session and the machine goes too, as Turbo off does.
                    if switched && !on {
                        tidy_up(&root, &card);
                    }
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
                    if switched {
                        notice.check(false);
                    }
                }
            });
            glib::ControlFlow::Continue
        });
    }

    // ── Live readings ───────────────────────────────────────────────────
    {
        let card = card.clone();
        let root = scroll.clone();
        // Read once, off the main thread: the CPU model and the GPUs do not
        // change while the application runs, and naming the GPUs reads the
        // PCI database. Until then (and if it fails) the tiles still run.
        let hw: Rc<RefCell<Option<std::sync::Arc<Hardware>>>> = Rc::new(RefCell::new(None));
        {
            let (hw, machine) = (Rc::clone(&hw), machine.clone());
            glib::spawn_future_local(async move {
                let Ok(read) = gio::spawn_blocking(|| {
                    let hw = Hardware::detect();
                    let line = machine_line(&hw);
                    (hw, line)
                })
                .await
                else {
                    machine.set_label(&i18n("Unknown CPU"));
                    return;
                };
                let (read, (line, tooltip)) = read;
                machine.set_label(&line);
                machine.set_tooltip_text(tooltip.as_deref());
                *hw.borrow_mut() = Some(std::sync::Arc::new(read));
            });
        }
        let tick = Cell::new(0u32);
        let net = net_tile.clone();
        let gpu_busy = Rc::new(Cell::new(false));
        let refresh = Refresh {
            update: Box::new(move |n| {
                if let Some(khz) = crate::views::details::telemetry::read_cpu_khz() {
                    #[allow(clippy::cast_precision_loss)]
                    let ghz = khz as f64 / 1_000_000.0;
                    cpu_tile.set(&format!("{ghz:.1} GHz"), ghz);
                }
                // Off the main thread, as Details reads it: NVIDIA's reading
                // loads NVML on first use and makes several calls per tick.
                if let Some(hw) = hw.borrow().clone().filter(|_| !gpu_busy.replace(true)) {
                    let running = crate::game_watch::current().and_then(|g| g.render_card);
                    let (tile, busy) = (gpu_tile.clone(), Rc::clone(&gpu_busy));
                    glib::spawn_future_local(async move {
                        let reading =
                            gio::spawn_blocking(move || gpu_reading(&hw, running.as_deref())).await;
                        busy.set(false);
                        if let Ok(Some((text, value))) = reading {
                            tile.set(&text, value);
                        }
                    });
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

/// Take away the preset and the Booster's changes a Turbo switched off from
/// outside left in force, off the main thread.
fn tidy_up(anchor: &gtk4::ScrolledWindow, card: &InfoCard) {
    let (anchor, card) = (anchor.clone(), card.clone());
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(turbo::tidy_up_blocking).await;
        match result {
            Ok(Ok(())) => {
                tracing::info!(target: "turbo", "Turbo went off from outside; its preset and Booster changes were put back");
            }
            Ok(Err(e)) => crate::widgets::toast::error(
                &anchor,
                &i18n("Some settings could not be put back"),
                &error_text(&e),
            ),
            Err(_) => crate::widgets::toast::error(
                &anchor,
                &i18n("Some settings could not be put back"),
                &i18n("the worker thread stopped"),
            ),
        }
        card.refresh_preset();
    });
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
        if !playing || n.is_multiple_of(IN_GAME_EVERY) {
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

/// The button's state, and whether Turbo is on, after a transition: from
/// the report, and from what systemd says now (`now`), which is Turbo's
/// state whatever the report expected.
fn finished_state(report: &Report, now: Option<bool>) -> (State, bool) {
    let (state, on) = expected_state(report);
    match now {
        // falcond came up although its start reported an error.
        Some(true) if !on => {
            let failed = report.count(Section::Failed).max(1);
            (
                State::Partial {
                    detail: ni18n(
                        "%n did not take effect — see details",
                        "%n did not take effect — see details",
                        failed,
                    ),
                },
                true,
            )
        }
        Some(false) if on => (
            State::Error {
                detail: ni18n(
                    "%n did not take effect — see details",
                    "%n did not take effect — see details",
                    report.count(Section::Failed).max(1),
                ),
            },
            false,
        ),
        _ => (state, on),
    }
}

/// [`finished_state`] from the report alone.
fn expected_state(report: &Report) -> (State, bool) {
    let failed = report.count(Section::Failed);
    let backend_item = report
        .items
        .iter()
        .find(|i| i.section == Section::Failed && i.kind == turbo::Kind::GameBackend);
    let backend_failed = backend_item.is_some();
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
        // What is wrong, when systemd said why ("falcond is not compatible
        // with this processor"); the report holds the rest.
        let detail = backend_item
            .and_then(|i| i.title.as_ref())
            .map_or_else(|| i18n("Per-game optimization could not be started"), tr);
        return (State::Error { detail }, false);
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
                // Turbo's state is falcond's unit, read back, not what the
                // transition meant to reach.
                let now = match turbo::state().await {
                    Ok(state) => Some(state == turbo::State::On),
                    Err(e) => {
                        tracing::warn!(error = %format!("{e:#}"), "Turbo's state could not be read back");
                        None
                    }
                };
                let _ = tx.send(match result {
                    Ok(report) => Event::Done(Box::new(report), now),
                    Err(e) => Event::Failed(error_text(&e)),
                });
            });
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the Turbo worker thread: {e}");
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
    /// The Turbo preset the session really holds, read off the main thread
    /// ([`turbo_preset::in_session`]); the record alone outlives a login.
    preset_in_session: Rc<Cell<Option<turbo_preset::Preset>>>,
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
            preset_in_session: Rc::new(Cell::new(None)),
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
                let mut flags = vec![Flag::new(Fact::Active, i18n("Turbo"))];
                if let Some(preset) = self.preset_in_session.get() {
                    flags.push(Flag::new(Fact::Active, i18n(preset.label())));
                }
                self.set_flags(&flags);
            } else {
                self.name.set_label(&i18n("Turbo is off"));
                self.facts
                    .set_label(&i18n("Games run without Big Game Mode's optimizations."));
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
            let app_id = g.steam_app_id.clone();
            let name = self.name.clone();
            let bare_name = g.display_name == g.process_name;
            let identity = g.clone();
            glib::spawn_future_local(async move {
                let found = gio::spawn_blocking(move || {
                    let steam = app_id.as_ref().and_then(|id| {
                        let home = std::env::var_os("HOME")?;
                        bigame_core::games::steam_cover(std::path::Path::new(&home), id)
                    });
                    // Only a process name: the library knows the title.
                    let installed = (steam.is_none() || bare_name)
                        .then(|| {
                            // Its own file first: two games can share a name.
                            bigame_core::games::installed_game_for_running(&identity)
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

    /// Read again which preset the session holds, and show it.
    fn refresh_preset(&self) {
        let me = self.clone();
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(turbo_preset::in_session).await;
            let preset = match read {
                Ok(Ok(preset)) => preset,
                Ok(Err(e)) => {
                    tracing::warn!(error = %format!("{e:#}"), "the session's environment could not be read");
                    None
                }
                Err(_) => None,
            };
            if me.preset_in_session.replace(preset) != preset && me.game.borrow().is_none() {
                me.show(None, me.turbo_on.get());
            }
        });
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

/// The machine in one line — its processor and the GPU games render on —
/// and a tooltip that says which GPU does what.
///
/// On a hybrid laptop, where another GPU drives the display and games are
/// sent to the discrete one, both are named, the games' first: `Intel Core
/// i7-13700H · NVIDIA GeForce RTX 4060 Laptop GPU + Intel UHD Graphics`.
/// Elsewhere only the games' GPU is: an idle integrated GPU beside a card
/// that drives the monitor would be noise.
fn machine_line(hw: &Hardware) -> (String, Option<String>) {
    use bigame_core::graphics::report::{GpuInfo, gpu_infos};
    let cpu = short_cpu(&hw.cpu);
    let (gpus, render) = gpu_infos(hw, None);
    let name = |g: &GpuInfo| {
        g.product
            .clone()
            .unwrap_or_else(|| vendor_gpu(g.vendor).unwrap_or_else(|| i18n("GPU")))
    };
    let Some(games) = render.and_then(|i| gpus.get(i)) else {
        return (cpu, None);
    };
    let games_name = name(games);
    let display = render
        .and_then(|i| bigame_core::hardware::offload_for(&hw.gpus, i).map(|_| i))
        .and_then(|i| {
            hw.gpus
                .iter()
                .enumerate()
                .find(|(j, g)| *j != i && !g.connected_outputs.is_empty())
                .and_then(|(j, _)| gpus.get(j))
        })
        .map(name)
        .filter(|n| *n != games_name);
    match display {
        Some(display) => (
            format!("{cpu} · {games_name} + {display}"),
            Some(
                i18n("Games render on %g; %d drives the display.")
                    .replace("%g", &games_name)
                    .replace("%d", &display),
            ),
        ),
        None => (
            format!("{cpu} · {games_name}"),
            Some(i18n("Games render on %s.").replace("%s", &games_name)),
        ),
    }
}

/// The processor's name as the pages show it
/// ([`bigame_core::hardware::cpu_display_name`]); with no name in
/// `/proc/cpuinfo`, its maker.
pub(crate) fn short_cpu(cpu: &bigame_core::hardware::Cpu) -> String {
    use bigame_core::hardware::{CpuVendor, UNKNOWN_CPU, cpu_display_name};
    if cpu.model != UNKNOWN_CPU {
        return cpu_display_name(&cpu.model);
    }
    match cpu.vendor {
        CpuVendor::Amd => i18n("%s processor").replace("%s", "AMD"),
        CpuVendor::Intel => i18n("%s processor").replace("%s", "Intel"),
        CpuVendor::Other => i18n(UNKNOWN_CPU),
    }
}

/// "NVIDIA GPU", for a GPU the PCI database does not name.
fn vendor_gpu(vendor: bigame_core::hardware::GpuVendor) -> Option<String> {
    let maker = match vendor {
        bigame_core::hardware::GpuVendor::Amd => "AMD",
        bigame_core::hardware::GpuVendor::Nvidia => "NVIDIA",
        bigame_core::hardware::GpuVendor::Intel => "Intel",
        bigame_core::hardware::GpuVendor::Other => return None,
    };
    Some(i18n("%s GPU").replace("%s", maker))
}

/// The card games use: the one the running game has open (`running`), else
/// the expected one. Its load when the driver reports it, else its
/// temperature. Blocking: from a worker thread.
fn gpu_reading(hw: &Hardware, running: Option<&str>) -> Option<(String, f64)> {
    let gpu =
        bigame_core::gpu_telemetry::games_gpu(&hw.gpus, running).and_then(|i| hw.gpus.get(i))?;
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
    fn an_unnamed_processor_is_named_by_its_maker() {
        let hw = Hardware::detect();
        let mut cpu = hw.cpu;
        cpu.model = "Intel(R) Core(TM) i7-12700K CPU @ 3.60GHz".into();
        assert_eq!(short_cpu(&cpu), "Intel Core i7-12700K");
        cpu.model = bigame_core::hardware::UNKNOWN_CPU.into();
        cpu.vendor = bigame_core::hardware::CpuVendor::Amd;
        assert_eq!(short_cpu(&cpu), "AMD processor");
        cpu.vendor = bigame_core::hardware::CpuVendor::Other;
        assert_eq!(short_cpu(&cpu), "Unknown CPU");
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
        let (state, on) = finished_state(&r, Some(false));
        assert!(!on);
        assert!(matches!(state, State::Error { .. }));
        // A start that reported an error but brought falcond up is on:
        // Turbo's state is the unit's.
        let (state, on) = finished_state(&r, Some(true));
        assert!(on);
        assert!(matches!(state, State::Partial { .. }));
    }

    #[test]
    fn a_falcond_that_cannot_run_here_says_why_on_home() {
        // Issue #4: the report names the cause, and Home shows it rather
        // than a generic failure.
        let mut r = report(&[]);
        r.items.push(turbo::Item {
            kind: turbo::Kind::GameBackend,
            section: Section::Failed,
            owner: "falcond".into(),
            detail: String::new(),
            text: None,
            title: Some(bigame_core::text::Text::plain(
                "falcond is not compatible with this processor",
            )),
        });
        let (state, on) = finished_state(&r, Some(false));
        assert!(!on);
        match state {
            State::Error { detail } => {
                assert_eq!(detail, "falcond is not compatible with this processor");
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn a_partial_failure_is_on_but_says_so() {
        let r = report(&[Section::Verified, Section::Failed]);
        let (state, on) = finished_state(&r, Some(true));
        assert!(on);
        assert!(matches!(state, State::Partial { .. }));
        // systemd unreadable: the report decides.
        assert!(finished_state(&r, None).1);
        // A Turbo on that reads off afterwards is not shown on.
        let (state, on) = finished_state(&report(&[Section::Verified]), Some(false));
        assert!(!on);
        assert!(matches!(state, State::Error { .. }));
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
