//! The Home screen.
//!
//! One decision drives this whole view: a beginner should be able to open the
//! application, press one thing, and go and play. Everything technical lives
//! behind Details, Tuning and Profiles.
//!
//! The engine runs on its own thread with its own Tokio runtime, and reports
//! back through a channel the GTK main loop drains. That keeps every privileged
//! call — which means D-Bus round trips and Polkit prompts — off the main
//! thread, so the window never stops redrawing while Booster works.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::booster::report::{Report, ReportState};
use bigame_core::booster::{BoosterEngine, Progress};
use bigame_core::hardware::Hardware;

use crate::i18n::i18n;
use crate::widgets::booster_button::{self, BoosterButton, State};

/// What the worker thread sends back to the UI.
enum Event {
    /// A pipeline stage began.
    Progress(Progress),
    /// Activation finished.
    Activated(Box<Report>),
    /// Activation could not start at all.
    Failed(String),
    /// Deactivation finished; carries the count that could not be restored.
    Deactivated(usize),
}

/// How often the live tiles refresh.
///
/// Two seconds is a deliberate compromise: fast enough that the numbers feel
/// live, slow enough that an idle Dashboard is not competing with the game for
/// CPU. The audit's DBUS-01 finding was a 500 ms poll that ran forever.
const TILE_REFRESH: std::time::Duration = std::time::Duration::from_secs(2);

/// Build the Home page.
///
/// Returns the page widget and a callback the window can use to show the
/// report, so navigation stays the window's concern rather than this view's.
#[must_use]
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn build(show_report: Rc<dyn Fn(&Report)>) -> gtk4::Widget {
    let button = BoosterButton::new();
    let last_report: Rc<RefCell<Option<Report>>> = Rc::new(RefCell::new(None));

    // ── Headline ────────────────────────────────────────────────────────
    let status = gtk4::Label::new(Some(&i18n("Checking your system…")));
    status.add_css_class("title-4");
    status.add_css_class("dim-label");
    status.set_wrap(true);
    status.set_justify(gtk4::Justification::Center);

    // ── Live tiles ──────────────────────────────────────────────────────
    let tiles = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
    tiles.set_halign(gtk4::Align::Center);
    let cpu_tile = Tile::new(&i18n("CPU"));
    let gpu_tile = Tile::new(&i18n("GPU"));
    let net_tile = Tile::new(&i18n("Network"));
    tiles.append(cpu_tile.widget());
    tiles.append(gpu_tile.widget());
    tiles.append(net_tile.widget());

    // ── Details link ────────────────────────────────────────────────────
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

    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 28);
    column.set_halign(gtk4::Align::Center);
    column.set_valign(gtk4::Align::Center);
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(18);
    column.set_margin_end(18);
    column.append(&status);
    column.append(button.widget());
    column.append(&tiles);
    column.append(&details);

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&column)
        .vexpand(true)
        .build();

    // ── Initial state ───────────────────────────────────────────────────
    // A previous session may have been killed while Booster was on. Recovery
    // runs first so the user is never shown "Ready" while the machine is still
    // carrying yesterday's changes.
    // A journal means a previous run left changes in force. Report the real
    // number from that record rather than a placeholder — "Active, 0
    // optimizations" is exactly the kind of statement this project is trying
    // to stop making. A record with nothing applied means nothing is in force.
    let initial_state = match BoosterEngine::active_summary() {
        Some(0) | None => State::Ready,
        Some(count) => State::Active { count },
    };
    button.set_state(&initial_state);
    booster_button::set_pulse(button.widget(), initial_state == State::Ready);

    // The Booster control is deliberately NOT given focus on startup.
    //
    // A focused button is the target of any activation the toolkit delivers —
    // Space, Enter, or anything a compositor or accessibility tool
    // synthesises — and this one changes system state. It was reproducible:
    // launching the window and taking a screenshot was enough to activate
    // Booster without anyone clicking it.
    //
    // Keyboard access is not lost. The button is focusable and sits in the tab
    // order like any other, so it is one Tab away; it simply is not armed
    // before the user has expressed any intent.

    // ── Activation ──────────────────────────────────────────────────────
    {
        let button = Rc::clone(&button);
        let status = status.clone();
        let details = details.clone();
        let last = Rc::clone(&last_report);
        let show = Rc::clone(&show_report);
        button.clone().connect_activated(move || {
            let turning_off = button.state().is_on();
            button.set_state(if turning_off {
                &State::Restoring
            } else {
                &State::Analyzing
            });
            booster_button::set_pulse(button.widget(), false);

            let (tx, rx) = mpsc::channel::<Event>();
            spawn_worker(tx, turning_off);

            let button = Rc::clone(&button);
            let status = status.clone();
            let details = details.clone();
            let last = Rc::clone(&last);
            let show = Rc::clone(&show);
            glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        Event::Progress(p) => {
                            if let Some(state) = progress_state(&p) {
                                button.set_state(&state);
                            }
                        }
                        Event::Activated(report) => {
                            apply_report(&button, &status, &details, &report);
                            let report = *report;
                            // Auto-open the report only when there is something
                            // to read: an already-optimal run has no changes to
                            // show, and forcing a page on the user for that
                            // would be noise.
                            let interesting = !report.applied.is_empty();
                            *last.borrow_mut() = Some(report);
                            if interesting {
                                if let Some(r) = last.borrow().as_ref() {
                                    show(r);
                                }
                            }
                            return glib::ControlFlow::Break;
                        }
                        Event::Deactivated(failed) => {
                            let state = if failed == 0 {
                                status.set_label(&i18n("Your previous settings are back"));
                                State::Ready
                            } else {
                                State::Error {
                                    detail: i18n("Some settings could not be restored"),
                                }
                            };
                            button.set_state(&state);
                            booster_button::set_pulse(button.widget(), state == State::Ready);
                            details.set_visible(false);
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

    // ── Live tiles + first status line ──────────────────────────────────
    {
        let status = status.clone();
        let cpu_tile = cpu_tile.clone();
        let gpu_tile = gpu_tile.clone();
        let net_tile = net_tile.clone();
        let first_run = std::cell::Cell::new(true);

        let refresh = move || {
            let hw = Hardware::detect();
            if first_run.replace(false) {
                status.set_label(&summary_line(&hw));
            }
            cpu_tile.set_value(&cpu_reading(&hw));
            gpu_tile.set_value(&gpu_reading(&hw));
            net_tile.set_value(&net_reading());
            glib::ControlFlow::Continue
        };
        // Populate immediately so the page is never blank, then keep it fresh.
        let refresh_now = refresh.clone();
        glib::idle_add_local_once(move || {
            refresh_now();
        });
        glib::timeout_add_local(TILE_REFRESH, refresh);
    }

    scroll.upcast()
}

/// Run the engine off the main thread.
///
/// A dedicated current-thread Tokio runtime is created here because the GTK
/// main loop is not a Tokio reactor, and every privileged call in the engine
/// goes through async zbus.
fn spawn_worker(tx: mpsc::Sender<Event>, turning_off: bool) {
    let spawned = std::thread::Builder::new()
        .name("bigame-booster".into())
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
                if turning_off {
                    match BoosterEngine::deactivate().await {
                        Ok(outcomes) => {
                            let failed = outcomes.iter().filter(|o| !o.status.is_ok()).count();
                            let _ = tx.send(Event::Deactivated(failed));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::Failed(format!("{e:#}")));
                        }
                    }
                    return;
                }

                let engine = BoosterEngine::detect();
                let progress_tx = tx.clone();
                match engine
                    .activate(|p| {
                        let _ = progress_tx.send(Event::Progress(p));
                    })
                    .await
                {
                    Ok(report) => {
                        let _ = tx.send(Event::Activated(Box::new(report)));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Failed(format!("{e:#}")));
                    }
                }
            });
        });

    if let Err(e) = spawned {
        tracing::error!("could not start the Booster worker thread: {e}");
    }
}

/// Translate an engine stage into a button state.
///
/// Returns `None` for stages with nothing new to say, so the label does not
/// flicker through states the user cannot read.
fn progress_state(progress: &Progress) -> Option<State> {
    match progress {
        Progress::DetectingHardware | Progress::DetectingCapabilities => Some(State::Analyzing),
        Progress::CapturingBaseline => Some(State::Optimizing {
            step: i18n("Recording your current settings"),
        }),
        Progress::Planning => Some(State::Optimizing {
            step: i18n("Deciding what is worth changing"),
        }),
        Progress::Applying { knob, index, total } => Some(State::Optimizing {
            step: format!("{knob}  ({index}/{total})"),
        }),
        Progress::Verifying { knob } => Some(State::Optimizing {
            step: format!("{} {knob}", i18n("Verifying")),
        }),
        Progress::Finished => None,
    }
}

/// Drive the button, status line and details link from a finished report.
fn apply_report(
    button: &Rc<BoosterButton>,
    status: &gtk4::Label,
    details: &gtk4::Button,
    report: &Report,
) {
    let state = match report.state() {
        ReportState::AlreadyOptimal => State::AlreadyOptimal,
        ReportState::Active => State::Active {
            count: report.verified_count(),
        },
        ReportState::Partial => State::Partial {
            applied: report.verified_count(),
            total: report.applied.len(),
        },
        ReportState::Failed => State::Error {
            detail: i18n("No change could be applied"),
        },
    };
    button.set_state(&state);
    booster_button::set_pulse(button.widget(), false);
    status.set_label(&report.headline());
    details.set_visible(!report.applied.is_empty() || !report.skipped.is_empty());
}

/// One-line description of the machine, shown before Booster has run.
fn summary_line(hw: &Hardware) -> String {
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
        bigame_core::hardware::GpuVendor::Amd => "AMD GPU".into(),
        bigame_core::hardware::GpuVendor::Nvidia => "NVIDIA GPU".into(),
        bigame_core::hardware::GpuVendor::Intel => "Intel GPU".into(),
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
    // Temperature is the number that matters while playing, and on this bench
    // it only reads correctly because the render GPU is selected by role — the
    // old code sampled the idle iGPU.
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

/// A labelled live value under the Booster control.
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
        // The caption already names the metric; without this a screen reader
        // reads two unrelated labels.
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
        // Collapsed whitespace, no leftover double spaces.
        assert!(!short_cpu("Intel(R)  Core(TM)  i5").contains("  "));
    }

    #[test]
    fn transient_stages_map_to_a_state_and_the_final_one_does_not() {
        assert_eq!(
            progress_state(&Progress::DetectingHardware),
            Some(State::Analyzing)
        );
        assert!(matches!(
            progress_state(&Progress::Planning),
            Some(State::Optimizing { .. })
        ));
        // Finished is handled by the report, not by a label flicker.
        assert_eq!(progress_state(&Progress::Finished), None);
    }

    #[test]
    fn applying_stage_shows_position_in_the_plan() {
        let state = progress_state(&Progress::Applying {
            knob: "GPU power level (card1)".into(),
            index: 2,
            total: 3,
        })
        .unwrap();
        let State::Optimizing { step } = state else {
            panic!("expected Optimizing");
        };
        assert!(step.contains("GPU power level (card1)"));
        assert!(step.contains("2/3"));
    }
}
