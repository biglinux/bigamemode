//! "Measure the difference" — the honest answer to "did that help?".
//!
//! Deliberately not part of pressing Turbo. Measuring launches the game
//! several times and takes minutes; doing that because someone pressed a
//! performance button would be worse than not measuring at all. It is offered
//! per game, from the card menu, and only for games that can be started
//! directly — a benchmark needs a handle on the process it is measuring, and
//! `steam -applaunch` returns immediately with the game running elsewhere.
//!
//! The dialog states the cost before starting, because "this will open your
//! game six times over the next four minutes" is not something to discover
//! halfway through.

use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::booster::BoosterEngine;
use bigame_core::booster::measure::{Arm, MeasureProgress, MeasurementPlan};
use bigame_core::text::Text;

use crate::i18n::{i18n, tr};

/// Seconds of frametime recorded per run.
const CAPTURE_SECONDS: u32 = 20;

/// Seconds to skip before recording, to get past menus and loading.
const START_DELAY_SECONDS: u32 = 14;

/// Runs per arm, including the discarded warm-up.
const RUNS_PER_ARM: usize = 3;

/// What the worker sends back.
enum Event {
    Progress(String),
    Done(Box<Result<Vec<Text>, String>>),
}

/// Ask whether to measure `game`, and do it if the answer is yes.
pub fn present(parent: &impl IsA<gtk4::Widget>, title: &str, command: &[String]) {
    // Whether there is anything to compare is known before any launch: on a
    // machine with nothing to change (no cpufreq, a virtual GPU) the plan is
    // empty, and six launches and five minutes would end in a failure.
    let anchor = parent.as_ref().clone();
    let title = title.to_owned();
    let command = command.to_vec();
    glib::spawn_future_local(async move {
        let nothing_to_change =
            gtk4::gio::spawn_blocking(|| BoosterEngine::detect().dry_run().1.is_empty())
                .await
                .unwrap_or(false);
        if nothing_to_change {
            let dialog = adw::AlertDialog::new(
                Some(&i18n("Nothing to measure")),
                Some(
                    &i18n(
                        "BiGame-mode would change nothing on this machine, so %t would \
                         be measured against itself. The game's profile is still \
                         applied by falcond whenever it runs.",
                    )
                    .replace("%t", &title),
                ),
            );
            dialog.add_response("close", &i18n("Close"));
            dialog.present(Some(&anchor));
        } else {
            ask(&anchor, &title, &command);
        }
    });
}

/// The confirmation, once there is a difference to measure.
fn ask(parent: &gtk4::Widget, title: &str, command: &[String]) {
    let runs = RUNS_PER_ARM * 2;
    let per_run = u64::from(CAPTURE_SECONDS + START_DELAY_SECONDS) + 10;
    #[allow(clippy::cast_possible_truncation)]
    let minutes = (per_run * runs as u64).div_ceil(60);

    let dialog = adw::AlertDialog::new(
        Some(&i18n("Measure the difference?")),
        Some(
            &i18n(
                "%t will be launched %n times — %h with your current settings and \
                 %h with the optimizations applied — for about %m minutes in total.\n\n\
                 Your settings are restored afterwards, including if something \
                 goes wrong. Leave the game in a scene that keeps rendering; a \
                 pause menu measures the pause menu.\n\n\
                 How the result is decided: runs alternate between the two \
                 configurations, the first run of each is discarded (a cold \
                 shader cache is unlike every run after it), and a difference \
                 counts only when it is larger than the variation between \
                 repeated runs of the same configuration and significant at \
                 95 %. Anything smaller is reported as no change, never as a \
                 small gain.",
            )
            .replace("%t", title)
            .replace("%n", &runs.to_string())
            .replace("%h", &RUNS_PER_ARM.to_string())
            .replace("%m", &minutes.to_string()),
        ),
    );
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("measure", &i18n("Measure"));
    dialog.set_response_appearance("measure", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let anchor = parent.clone();
    let command = command.to_vec();
    let title = title.to_owned();
    {
        let anchor = anchor.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "measure" {
                run(&anchor, &title, &command);
            }
        });
    }
    dialog.present(Some(&anchor));
}

/// Run the measurement, showing progress and then the result.
fn run(parent: &gtk4::Widget, title: &str, command: &[String]) {
    let progress = adw::AlertDialog::new(Some(&i18n("Measuring…")), Some(&i18n("Starting")));
    // No cancel response: stopping midway would leave an arm applied, and the
    // engine restores the baseline only when it finishes or fails.
    progress.present(Some(parent));

    let (tx, rx) = mpsc::channel::<Event>();
    spawn_worker(tx, command.to_vec());

    let progress_ref = progress.clone();
    let parent = parent.clone();
    let title = title.to_owned();
    glib::timeout_add_local(Duration::from_millis(120), move || {
        loop {
            let event = match rx.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                // The worker ended without an answer (it panicked).
                Err(mpsc::TryRecvError::Disconnected) => Event::Done(Box::new(Err(i18n(
                    "The measurement stopped without an answer. Open Logs to see why.",
                )))),
            };
            match event {
                Event::Progress(text) => progress_ref.set_body(&text),
                Event::Done(result) => {
                    progress_ref.close();
                    show_result(&parent, &title, *result);
                    return glib::ControlFlow::Break;
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn spawn_worker(tx: mpsc::Sender<Event>, command: Vec<String>) {
    let spawned = std::thread::Builder::new()
        .name("bigame-measure".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Event::Done(Box::new(Err(e.to_string()))));
                    return;
                }
            };

            runtime.block_on(async {
                let plan = MeasurementPlan {
                    command,
                    duration_s: CAPTURE_SECONDS,
                    runs_per_arm: RUNS_PER_ARM,
                    start_delay_s: START_DELAY_SECONDS,
                };
                let log_dir = log_directory();
                let engine = BoosterEngine::detect();
                let progress_tx = tx.clone();

                let result = engine
                    .measure(&plan, &log_dir, |p| {
                        let _ = progress_tx.send(Event::Progress(describe(&p)));
                    })
                    .await;

                let payload = match result {
                    Ok(measurement) => Ok(measurement
                        .outcomes
                        .iter()
                        .map(bigame_core::booster::report::Outcome::describe_text)
                        .collect()),
                    Err(e) => Err(format!("{e:#}")),
                };
                let _ = tx.send(Event::Done(Box::new(payload)));
            });
        });

    if let Err(e) = spawned {
        tracing::error!("could not start the measurement thread: {e}");
    }
}

/// Where `MangoHud` writes its captures.
fn log_directory() -> std::path::PathBuf {
    let base = bigame_core::paths::cache_home();
    base.join("bigame-mode").join("benchmark")
}

/// Turn an engine stage into something worth reading.
fn describe(progress: &MeasureProgress) -> String {
    match progress {
        MeasureProgress::Switching { to } => match to {
            Arm::Baseline => i18n("Restoring your current settings"),
            Arm::Optimized => i18n("Applying the optimizations"),
        },
        MeasureProgress::Running {
            arm,
            run,
            total,
            warmup,
        } => {
            let which = match arm {
                Arm::Baseline => i18n("current settings"),
                Arm::Optimized => i18n("optimized"),
            };
            if *warmup {
                // Saying it will be discarded avoids the impression that the
                // first run counted and something went wrong.
                i18n("Run %r of %t — %w (warm-up, discarded)")
                    .replace("%r", &run.to_string())
                    .replace("%t", &total.to_string())
                    .replace("%w", &which)
            } else {
                i18n("Run %r of %t — %w")
                    .replace("%r", &run.to_string())
                    .replace("%t", &total.to_string())
                    .replace("%w", &which)
            }
        }
        MeasureProgress::Analysing => i18n("Analysing"),
    }
}

fn show_result(parent: &gtk4::Widget, title: &str, result: Result<Vec<Text>, String>) {
    let (heading, body) = match result {
        Ok(lines) if lines.is_empty() => (
            i18n("Nothing could be compared"),
            i18n("The runs produced no usable frametimes."),
        ),
        Ok(lines) => (
            i18n("Results for %t").replace("%t", title),
            format!(
                "{}\n\n{}",
                lines.iter().map(tr).collect::<Vec<_>>().join("\n"),
                i18n(
                    "Each metric is judged against how much that same metric varied \
                     between runs of your current settings. Anything smaller than \
                     that variation is reported as no change, because it is not \
                     evidence of one."
                )
            ),
        ),
        Err(error) => (i18n("Measurement failed"), error),
    };

    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
    dialog.add_response("close", &i18n("Close"));
    dialog.set_default_response(Some("close"));
    dialog.present(Some(parent));
}
