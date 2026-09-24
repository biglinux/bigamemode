//! The Home screen.
//!
//! One decision drives this view: a beginner should be able to open the
//! application, press one thing, and go and play. Turbo is that one thing —
//! the master switch. Off, BiGame-mode does not intervene in games; on, it
//! detects them and optimizes them.
//!
//! While a game runs, Home says which one, how it runs, and which profile is
//! in force, with one line summarising what Turbo did and a link to the full
//! report. Everything else lives behind that link.
//!
//! Transitions run on a worker thread with its own Tokio runtime and report
//! back through a channel the GTK main loop drains, so D-Bus round trips and
//! Polkit prompts never stall the window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::hardware::Hardware;
use bigame_core::running::GameIdentity;
use bigame_core::turbo::{self, Report, Section, Step};

use crate::i18n::{i18n, ni18n};
use crate::widgets::booster_button::{self, BoosterButton, State};

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
    status.add_css_class("title-4");
    status.add_css_class("dim-label");
    status.set_wrap(true);
    status.set_justify(gtk4::Justification::Center);

    let game = GameCard::new();

    // ── Summary + details link ──────────────────────────────────────────
    let summary = gtk4::Label::new(None);
    summary.add_css_class("dim-label");
    summary.set_wrap(true);
    summary.set_justify(gtk4::Justification::Center);
    summary.set_visible(false);
    let details = gtk4::Button::builder()
        .label(i18n("View optimization details"))
        .css_classes(["flat"])
        .halign(gtk4::Align::Center)
        .visible(false)
        .build();
    {
        let last = Rc::clone(&last_report);
        let show = Rc::clone(&show_report);
        details.connect_clicked(move |_| {
            if let Some(report) = last.borrow().as_ref() {
                show(report);
            }
        });
    }

    // ── Live tiles ──────────────────────────────────────────────────────
    let tiles = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    tiles.set_halign(gtk4::Align::Center);
    let cpu_tile = Tile::new(&i18n("CPU"));
    let gpu_tile = Tile::new(&i18n("GPU"));
    let net_tile = Tile::new(&i18n("Network"));
    tiles.append(cpu_tile.widget());
    tiles.append(gpu_tile.widget());
    tiles.append(net_tile.widget());

    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    column.set_halign(gtk4::Align::Center);
    column.set_valign(gtk4::Align::Center);
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(18);
    column.set_margin_end(18);
    column.append(&status);
    column.append(button.widget());
    column.append(game.widget());
    column.append(&summary);
    column.append(&details);
    column.append(&tiles);

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&column)
        .vexpand(true)
        .build();

    let turbo_on = Rc::new(Cell::new(false));
    let show_summary = {
        let summary = summary.clone();
        let details = details.clone();
        let last = Rc::clone(&last_report);
        let turbo_on = Rc::clone(&turbo_on);
        Rc::new(move || {
            let text = last
                .borrow()
                .as_ref()
                .filter(|r| r.turned_on && turbo_on.get())
                .map(summary_line);
            summary.set_visible(text.as_ref().is_some_and(|t| !t.is_empty()));
            details.set_visible(last.borrow().is_some());
            if let Some(text) = text {
                summary.set_label(&text);
            }
        })
    };

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
        let show_summary = Rc::clone(&show_summary);
        glib::spawn_future_local(async move {
            let state = gtk4::gio::spawn_blocking(turbo::state_blocking).await;
            let on = matches!(state, Ok(Ok(turbo::State::On)));
            turbo_on.set(on);
            let state = if on {
                State::On {
                    detail: on_detail(crate::game_watch::current().as_ref()),
                }
            } else {
                State::Off
            };
            button.set_state(&state);
            booster_button::set_pulse(button.widget(), !on);
            show_summary();
        });
    }

    // ── The running game ────────────────────────────────────────────────
    {
        let game = game.clone();
        let button = Rc::clone(&button);
        let turbo_on = Rc::clone(&turbo_on);
        crate::game_watch::subscribe(move |current| {
            game.show(current, turbo_on.get());
            if turbo_on.get() && matches!(button.state(), State::On { .. }) {
                button.set_state(&State::On {
                    detail: on_detail(current),
                });
            }
        });
    }

    // ── Activation ──────────────────────────────────────────────────────
    {
        let button = Rc::clone(&button);
        let status = status.clone();
        let last = Rc::clone(&last_report);
        let show = Rc::clone(&show_report);
        let turbo_on = Rc::clone(&turbo_on);
        let show_summary = Rc::clone(&show_summary);
        let game = game.clone();
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
            let show_summary = Rc::clone(&show_summary);
            let game = game.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        Event::Step(step) => {
                            if !turning_off {
                                button.set_state(&State::Working {
                                    step: step_text(&step),
                                });
                            }
                        }
                        Event::Done(report) => {
                            let report = *report;
                            let (state, on) = finished_state(&report);
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
                            show_summary();
                            game.show(crate::game_watch::current().as_ref(), on);
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
        let show_summary = Rc::clone(&show_summary);
        let game = game.clone();
        let root = scroll.clone();
        let systemd = bigame_core::systemd::Reader::system();
        glib::timeout_add_local(std::time::Duration::from_secs(10), move || {
            if !root.is_mapped() || !button.state().is_interactive() {
                return glib::ControlFlow::Continue;
            }
            let Some(unit) = systemd
                .as_ref()
                .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
            else {
                return glib::ControlFlow::Continue;
            };
            let on = if unit.is_installed() {
                unit.is_active()
            } else {
                turbo_on.get()
            };
            if on != turbo_on.get()
                || Report::load_last().map(|r| r.at) != last.borrow().as_ref().map(|r| r.at)
            {
                turbo_on.set(on);
                *last.borrow_mut() = Report::load_last();
                button.set_state(&if on {
                    State::On {
                        detail: on_detail(crate::game_watch::current().as_ref()),
                    }
                } else {
                    State::Off
                });
                booster_button::set_pulse(button.widget(), !on);
                show_summary();
                game.show(crate::game_watch::current().as_ref(), on);
            }
            glib::ControlFlow::Continue
        });
    }

    // ── Live readings ───────────────────────────────────────────────────
    {
        let status = status.clone();
        let game = game.clone();
        let root = scroll.clone();
        // Probed once: the CPU model and render GPU do not change while the
        // application runs, and re-probing every tick was a full hardware
        // scan to update three numbers.
        let hw = Rc::new(Hardware::detect());
        status.set_label(&summary_line_machine(&hw));
        let tick = Cell::new(0u32);
        let refresh = Refresh {
            update: Box::new(move || {
                cpu_tile.set_value(&cpu_reading(&hw));
                gpu_tile.set_value(&gpu_reading(&hw));
                net_tile.set_value(&net_reading());
                game.tick();
            }),
            root,
            tick,
        };
        // Once as soon as the page is shown, then on the timer -- otherwise
        // the tiles read "—" until the first tick, ten seconds into a game.
        let refresh = std::rc::Rc::new(refresh);
        {
            let refresh = std::rc::Rc::clone(&refresh);
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
    update: Box<dyn Fn()>,
    root: gtk4::ScrolledWindow,
    tick: Cell<u32>,
}

impl Refresh {
    fn force(&self) {
        (self.update)();
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
            (self.update)();
        }
        glib::ControlFlow::Continue
    }
}

/// The button's line while Turbo is on.
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
                    Err(e) => Event::Failed(format!("{e:#}")),
                });
            });
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the Turbo worker thread: {e}");
    }
}

// ── The game card ────────────────────────────────────────────────────────────

/// The running game: cover, name, how long, how it runs, and its profile.
#[derive(Clone)]
struct GameCard {
    root: gtk4::Box,
    cover: gtk4::Image,
    name: gtk4::Label,
    running: gtk4::Label,
    facts: gtk4::Label,
    profile: gtk4::Label,
    ai: gtk4::Label,
    create: gtk4::Button,
    game: Rc<RefCell<Option<GameIdentity>>>,
    turbo_on: Rc<Cell<bool>>,
}

impl GameCard {
    fn new() -> Self {
        // A fixed-size image, not a Picture: a Picture asks for the art's
        // natural size (600x900 for Steam's covers) and stretches the card.
        let cover = gtk4::Image::new();
        cover.set_pixel_size(120);
        cover.set_valign(gtk4::Align::Center);

        let name = gtk4::Label::new(None);
        name.add_css_class("title-3");
        name.set_xalign(0.0);
        name.set_wrap(true);
        let running = gtk4::Label::new(None);
        running.add_css_class("dim-label");
        running.set_xalign(0.0);
        let facts = gtk4::Label::new(None);
        facts.add_css_class("caption");
        facts.set_xalign(0.0);
        facts.set_wrap(true);
        let profile = gtk4::Label::new(None);
        profile.set_xalign(0.0);
        profile.set_wrap(true);
        let ai = gtk4::Label::new(None);
        ai.set_xalign(0.0);
        ai.set_wrap(true);
        ai.set_visible(false);
        let create = gtk4::Button::builder()
            .label(i18n("Create profile"))
            .css_classes(["pill", "suggested-action"])
            .halign(gtk4::Align::Start)
            .visible(false)
            .build();
        create.set_action_name(Some("app.profile-review"));
        // The action takes the process name. Until a game is running there is
        // none, but without a target of the right type GTK rejects the
        // button on every update ("parameter type mismatch").
        create.set_action_target_value(Some(&glib::variant::ToVariant::to_variant("")));

        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        text.set_valign(gtk4::Align::Center);
        text.append(&name);
        text.append(&running);
        text.append(&facts);
        text.append(&profile);
        text.append(&ai);
        text.append(&create);

        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        root.add_css_class("card");
        root.set_halign(gtk4::Align::Center);
        root.set_margin_start(6);
        root.set_margin_end(6);
        for side in [
            &cover.clone().upcast::<gtk4::Widget>(),
            &text.clone().upcast(),
        ] {
            side.set_margin_top(12);
            side.set_margin_bottom(12);
        }
        cover.set_margin_start(12);
        text.set_margin_end(16);
        root.append(&cover);
        root.append(&text);
        root.set_visible(false);

        Self {
            root,
            cover,
            name,
            running,
            facts,
            profile,
            ai,
            create,
            game: Rc::new(RefCell::new(None)),
            turbo_on: Rc::new(Cell::new(false)),
        }
    }

    fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    fn show(&self, game: Option<&GameIdentity>, turbo_on: bool) {
        *self.game.borrow_mut() = game.cloned();
        self.turbo_on.set(turbo_on);
        let Some(g) = game else {
            self.root.set_visible(false);
            return;
        };
        self.name.set_label(&g.display_name);
        let cover = g.steam_app_id.as_ref().and_then(|id| {
            let home = std::env::var_os("HOME")?;
            bigame_core::games::steam_cover(std::path::Path::new(&home), id)
        });
        self.cover.set_visible(cover.is_some());
        if let Some(path) = cover {
            self.cover.set_from_file(Some(&path));
        }
        let mut facts = vec![match &g.runtime {
            bigame_core::running::Runtime::Native => i18n("Native"),
            bigame_core::running::Runtime::Proton(tool) if !tool.is_empty() => tool.clone(),
            bigame_core::running::Runtime::Proton(_) => "Proton".into(),
            bigame_core::running::Runtime::Wine => "Wine".into(),
        }];
        if g.graphics != bigame_core::running::Graphics::Unknown {
            facts.push(g.graphics.label().to_owned());
        }
        facts.push(g.process_name.clone());
        self.facts.set_label(&facts.join(" · "));
        self.create
            .set_action_target_value(Some(&glib::variant::ToVariant::to_variant(&g.process_name)));
        self.root.set_visible(true);
        self.tick();
    }

    /// Refresh what changes while the game runs.
    fn tick(&self) {
        let Some(g) = self.game.borrow().clone() else {
            return;
        };
        if let Some(secs) = bigame_core::running::running_for(g.pid) {
            self.running.set_label(&format!(
                "{} · {:02}:{:02}:{:02}",
                i18n("Running"),
                secs / 3600,
                (secs / 60) % 60,
                secs % 60
            ));
        }
        // What AI Graphics is really doing in the game, from what it loaded
        // and OptiScaler's own log — hidden when nothing was installed.
        match bigame_core::graphics::status_running(&g) {
            Some(st) => {
                self.ai.set_label(&format!(
                    "{} · {}",
                    i18n("AI Graphics"),
                    crate::views::ai_graphics::status_text(&st)
                ));
                self.ai.set_visible(true);
            }
            None => self.ai.set_visible(false),
        }
        if !self.turbo_on.get() {
            self.profile
                .set_label(&i18n("Turbo is off, so this game is not being optimized"));
            self.create.set_visible(false);
            return;
        }
        let active = bigame_core::status::read().and_then(|s| s.active_profile);
        let (text, offer) = match active.as_deref() {
            Some("Proton") => (
                i18n("No profile of its own yet · using falcond's general Proton profile"),
                true,
            ),
            Some(name) => (i18n("Profile %s").replace("%s", name), false),
            None => (i18n("No profile is active for this game"), true),
        };
        self.profile.set_label(&text);
        self.create.set_visible(offer);
    }
}

// ── Readings ─────────────────────────────────────────────────────────────────

/// One-line description of the machine.
fn summary_line_machine(hw: &Hardware) -> String {
    let gpu = hw
        .render_gpu()
        .map_or_else(String::new, |g| format!(" · {}", short_gpu(g)));
    format!("{}{}", short_cpu(&hw.cpu.model), gpu)
}

/// Trim vendor boilerplate so the line stays readable at small widths.
fn short_cpu(model: &str) -> String {
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

fn cpu_reading(hw: &Hardware) -> String {
    let khz: Option<u64> =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
            .ok()
            .and_then(|s| s.trim().parse().ok());
    match khz {
        #[allow(clippy::cast_precision_loss)]
        Some(k) => format!("{:.1} GHz", k as f64 / 1_000_000.0),
        None => hw.cpu.current_governor.clone().unwrap_or_else(|| i18n("—")),
    }
}

fn gpu_reading(hw: &Hardware) -> String {
    let Some(gpu) = hw.render_gpu() else {
        return i18n("—");
    };
    match gpu.hwmon_u64("temp1_input") {
        Some(milli) => format!("{} °C", milli / 1000),
        None => gpu
            .busy_percent()
            .map_or_else(|| i18n("—"), |b| format!("{b}%")),
    }
}

fn net_reading() -> String {
    bigame_core::network::primary_link().map_or_else(
        || i18n("Offline"),
        |l| match l.speed_mbps {
            Some(mbps) => format!("{mbps} Mb/s"),
            None => l.name,
        },
    )
}

/// A labelled live value.
#[derive(Clone)]
struct Tile {
    root: gtk4::Box,
    value: gtk4::Label,
}

impl Tile {
    fn new(label: &str) -> Self {
        let value = gtk4::Label::new(Some("—"));
        value.add_css_class("title-3");

        let caption = gtk4::Label::new(Some(label));
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        root.set_halign(gtk4::Align::Center);
        root.set_width_request(96);
        root.append(&value);
        root.append(&caption);
        root.update_property(&[gtk4::accessible::Property::Label(label)]);

        Self { root, value }
    }

    fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    fn set_value(&self, text: &str) {
        self.value.set_label(text);
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
}
