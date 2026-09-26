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
        "profiles" => page(
            i18n("Profiles"),
            &[
                i18n(
                    "<b>Game library</b> — Games from Steam, Lutris, Heroic and the application menu, and the profiles that tune them. A green dot is your own profile; blue is one that ships with falcond.",
                ),
                i18n(
                    "<b>Card menu (⋮)</b> — Launch (Turbo) starts the game with BiGame-mode's launch settings; Create with Wizard explains every option; AI Graphics, Measure the difference, Edit, Restore the game's graphics, Delete.",
                ),
                i18n(
                    "<b>Profile</b> — What falcond applies while the game runs: performance mode, sched-ext scheduler, V-Cache mode and screen-saver inhibit, plus Gamescope and MangoHud for launches from BiGame-mode.",
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
                    "<b>System performance</b> — What falcond applies while a game runs: performance mode, the sched-ext scheduler for games without one of their own, 3D V-Cache, and falcond's own settings.",
                ),
                i18n(
                    "<b>Display and Gamescope</b> — Games started from BiGame-mode run inside Gamescope, with a filter, sharpness and render and output sizes.",
                ),
                i18n(
                    "<b>Upscaling and sharpening</b> — Wine FSR for Proton games in exclusive fullscreen, and vkBasalt's visual filters. Two upscalers on at once are named, with a way out.",
                ),
                i18n(
                    "<b>Frame generation</b> — lsfg-vk: the global switch, your Lossless.dll, and each game's multiplier. It raises the presented frame rate, not the rendered one.",
                ),
                i18n(
                    "<b>Advanced</b> — sched-ext availability, the Gamescope options the installed version accepts, the environment file.",
                ),
            ],
        ),
        "dashboard" => page(
            i18n("Details"),
            &[
                i18n(
                    "<b>Overview</b> — One line on how the machine stands, and a chip per item: Turbo, falcond, the profile, power, the scheduler, the GPU, Gamescope, upscaling, frame generation.",
                ),
                i18n(
                    "<b>Telemetry and graphics cards</b> — CPU and GPU readings, and one card per GPU with its load, clock, VRAM, temperature and power; which one renders the game.",
                ),
                i18n(
                    "<b>Performance and video pipeline</b> — Each item says whether it is active, waiting, configured but not detected, off, missing or not supported. Open a row for what it means, the evidence, and the fix.",
                ),
                i18n(
                    "<b>Problems</b> — Everything that needs attention, classed fixable, needs you, hardware or information, with commands to copy. Hardware limits are never errors.",
                ),
                i18n(
                    "<b>Network, background load, Steam launch options, support report</b> — The connection and a DNS comparison; programs competing for the CPU; launch options that call a missing program; a report for support with your name, home folder and host masked.",
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
