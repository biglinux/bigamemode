//! Context-aware tutorial dialogs explaining each view's purpose and options.

use adw::prelude::*;
use libadwaita as adw;

use crate::i18n::i18n;

/// Show a tutorial dialog for the currently active tab.
///
/// `tab_name` is the `AdwViewStack` child name of the page.
pub fn show(widget: &impl IsA<gtk4::Widget>, tab_name: &str) {
    let (heading, body) = content(tab_name);

    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .body_use_markup(true)
        .close_response("close")
        .default_response("close")
        .build();

    dialog.add_response("close", &i18n("Got It"));

    // Present attached to the top-level window so it's modal
    let win = widget.root().and_downcast::<gtk4::Window>();
    dialog.present(win.as_ref());
}

/// Return `(heading, body_markup)` for a given tab name.
// One entry per page; splitting the table would only scatter it.
#[allow(clippy::too_many_lines)]
fn content(tab: &str) -> (String, String) {
    let page = |heading: String, items: &[String]| (heading, items.join("\n\n"));
    match tab {
        "home" => page(
            i18n("Home"),
            &[
                i18n(
                    "<b>Turbo</b> — The main switch. Off, BiGame-mode does not intervene in games. On, falcond runs and applies each game's profile when it starts, and undoes it when the game closes.",
                ),
                i18n(
                    "<b>Running game</b> — The game BiGame-mode sees right now, how it runs (native, Proton or Wine) and which profile is active. <b>Create profile</b> appears when it has none.",
                ),
                i18n(
                    "<b>Optimization details</b> — What Turbo applied, what it left to the component that owns it, and why anything was skipped.",
                ),
            ],
        ),
        "dashboard" => page(
            i18n("Details"),
            &[
                i18n(
                    "<b>Real-time telemetry</b> — CPU and GPU frequency, GPU temperature, RAM, disk activity and network latency.",
                ),
                i18n(
                    "<b>Performance</b> — The power profile, Turbo's state and lsfg-vk frame generation.",
                ),
                i18n(
                    "<b>Video runtime status</b> — Whether Gamescope, Wine FSR, vkBasalt and frame generation are really active in the running game, not only enabled in settings.",
                ),
                i18n(
                    "<b>falcond</b> — The active sched-ext scheduler, the V-Cache mode and the game profile falcond has applied.",
                ),
                i18n(
                    "<b>Detected games</b> — Create a profile with the wizard, or launch a game with BiGame-mode's video settings.",
                ),
            ],
        ),
        "profiles" => page(
            i18n("Game Profiles"),
            &[
                i18n(
                    "<b>Game library</b> — Games from Steam, Lutris, Heroic and the application menu, and the profiles that tune them. A green dot is your own profile; blue is one that ships with falcond.",
                ),
                i18n(
                    "<b>Profile</b> — What falcond applies while the game runs: performance mode, sched-ext scheduler, V-Cache mode and screen-saver inhibit, plus Gamescope and MangoHud for launches from BiGame-mode.",
                ),
                i18n("<b>Wizard (+)</b> — A guided profile that explains every option."),
                i18n(
                    "<b>Card menu (⋮)</b> — AI Graphics for the game, Measure the difference, Edit or Delete the profile.",
                ),
                i18n(
                    "<b>Import</b> — Load a .conf or .toml profile from disk. Drag-and-drop onto the list also works.",
                ),
            ],
        ),
        "tuning" => page(
            i18n("Tuning"),
            &[
                i18n(
                    "<b>Daemon</b> — falcond's global settings: performance mode and how often it scans for new games.",
                ),
                i18n(
                    "<b>Scheduler</b> — The sched-ext CPU scheduler falcond loads for games without a scheduler of their own, and its tuning preset. Needs scx-tools.",
                ),
                i18n("<b>V-Cache</b> — AMD Ryzen X3D only: which CCD games prefer."),
                i18n(
                    "<b>Device mode</b> — Which of falcond's profile sets is used: desktop, handheld or HTPC.",
                ),
                i18n(
                    "<b>CPU governor</b> — Shown for reference; power-profiles-daemon sets it through the power profile.",
                ),
                i18n(
                    "<b>Advanced</b> — Scheduler flags, the schedulers installed, and the Gamescope options the installed version accepts.",
                ),
            ],
        ),
        "video" => page(
            i18n("Advanced Video Settings"),
            &[
                i18n(
                    "<b>Gamescope upscaling</b> — Runs games launched from BiGame-mode inside Gamescope, rendering at a lower resolution and upscaling with FSR, NIS or integer scaling.",
                ),
                i18n(
                    "<b>Wine/Proton FSR</b> — Wine's own fullscreen FSR, for games in exclusive fullscreen.",
                ),
                i18n(
                    "<b>vkBasalt</b> — Vulkan post-processing, such as CAS sharpening, from a vkBasalt configuration file.",
                ),
                i18n(
                    "<b>Frame generation</b> — lsfg-vk, when it is installed and your Lossless.dll is configured. It raises the presented frame rate, not the rendered one, and adds latency.",
                ),
                i18n(
                    "<b>No doubling up</b> — A game with AI Graphics installed runs without Wine FSR and Gamescope upscaling, and without lsfg-vk when OptiScaler generates frames.",
                ),
            ],
        ),
        "benchmark" => page(
            i18n("Benchmark"),
            &[
                i18n(
                    "<b>Workloads</b> — The benchmarks this machine can run, and what is missing when one cannot.",
                ),
                i18n(
                    "<b>What measurement found</b> — Settings measured on this machine, and whether they helped, hurt or made no difference.",
                ),
                i18n(
                    "<b>How results are decided</b> — Runs alternate between configurations, the first of each is discarded, and a difference counts only when it exceeds the run-to-run variation and passes a statistical test.",
                ),
                i18n(
                    "<b>Measure the difference</b> — From a game's card menu, for games that start directly.",
                ),
            ],
        ),
        "diagnostics" => page(
            i18n("Diagnostics"),
            &[
                i18n(
                    "<b>System health</b> — Whether everything games need is present and working, with the fix for anything that is not.",
                ),
                i18n("<b>AI Graphics</b> — Every game BiGame-mode has placed files in."),
                i18n(
                    "<b>Background load</b> — Programs competing with the game for the CPU. Reported, never changed.",
                ),
                i18n(
                    "<b>Network</b> — The connection in use, its queue discipline and a DNS resolver comparison.",
                ),
                i18n(
                    "<b>Support report</b> — Copy or save a report for support, with your user name, home folder and host masked.",
                ),
            ],
        ),
        "logs" => page(
            i18n("Logs"),
            &[
                i18n(
                    "<b>One view</b> — falcond, BiGame-mode, power-profiles-daemon, scx_loader, Gamescope and the kernel's GPU messages, read from the system journal.",
                ),
                i18n(
                    "<b>Filter and search</b> — Show only errors, warnings or one source, or search the text.",
                ),
                i18n(
                    "<b>Follow</b> — New entries appear while the page is open; nothing is read while it is not.",
                ),
                i18n(
                    "<b>Copy and export</b> — Copy what is shown, or save it to a file with your user name, home folder and host masked.",
                ),
            ],
        ),
        "settings" => page(
            i18n("Settings"),
            &[
                i18n(
                    "<b>Start in the background</b> — At login, in the tray, so games started from Steam are noticed with the window closed.",
                ),
                i18n(
                    "<b>Game profiles</b> — Whether a profile is offered when a game without one starts.",
                ),
                i18n(
                    "<b>Notifications</b> and <b>ping target</b> — Notifications when a game starts or exits, and the address the Details page pings for its latency graph.",
                ),
                i18n(
                    "<b>Profiles from an older BiGame-mode</b> — Profiles that can never match their game, and the fix.",
                ),
                i18n(
                    "<b>Hand falcond back</b> — Return falcond to exactly the state it was in before BiGame-mode first changed it.",
                ),
            ],
        ),
        _ => (i18n("Help"), i18n("No help available for this view.")),
    }
}
