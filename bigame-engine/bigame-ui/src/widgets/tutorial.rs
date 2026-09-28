//! Help for the page on screen: what it is for, how to use it, and what
//! each part of it does.
//!
//! Every page's help has the same shape, so it is learnt once: a sentence
//! on what the page is for, the steps of its usual task when it has one,
//! one row per part of the page (in the order they appear), and a tip. Each
//! line names a control by the label the page gives it, and says only what
//! that control does.

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::turbo_preset;

use crate::i18n::i18n;
use crate::widgets::turbo_presets;
use crate::widgets::welcome::feature_row;

/// One page's help.
struct Help {
    /// The page's icon, as in the sidebar.
    icon: &'static str,
    title: String,
    /// What the page is for, in a sentence.
    lead: String,
    /// The usual task, in order; empty for a page that has none.
    steps: Vec<String>,
    /// One per part of the page: icon, name, what it does.
    parts: Vec<(&'static str, String, String)>,
    tip: String,
}

/// Show the help for the page named `tab` (its `AdwViewStack` child name).
pub fn show(widget: &impl IsA<gtk4::Widget>, tab: &str) {
    let help = content(tab);

    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    page.set_margin_start(24);
    page.set_margin_end(24);
    page.set_margin_top(12);
    page.set_margin_bottom(24);

    let icon = gtk4::Image::from_icon_name(help.icon);
    icon.set_pixel_size(48);
    icon.add_css_class("help-page-icon");
    let intro = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    intro.append(&icon);
    intro.append(
        &gtk4::Label::builder()
            .label(&help.title)
            .css_classes(["title-2"])
            .build(),
    );
    intro.append(
        &gtk4::Label::builder()
            .label(&help.lead)
            .wrap(true)
            .justify(gtk4::Justification::Center)
            .css_classes(["dim-label"])
            .build(),
    );
    page.append(&intro);

    if !help.steps.is_empty() {
        page.append(&heading(&i18n("Step by step")));
        let steps = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        for (n, step) in (1..).zip(&help.steps) {
            steps.append(&step_row(n, step));
        }
        page.append(&steps);
    }

    page.append(&heading(&i18n("On this page")));
    let parts = gtk4::Box::new(gtk4::Orientation::Vertical, 14);
    for (icon, title, text) in &help.parts {
        parts.append(&feature_row(icon, title, text, 24));
    }
    page.append(&parts);

    let tip_icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
    tip_icon.set_valign(gtk4::Align::Start);
    let tip = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    tip.add_css_class("help-tip");
    tip.append(&tip_icon);
    tip.append(
        &gtk4::Label::builder()
            .label(&help.tip)
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    page.append(&tip);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&page)
        .build();

    let got_it = gtk4::Button::builder()
        .label(i18n("Got It"))
        .halign(gtk4::Align::Center)
        .margin_top(12)
        .margin_bottom(12)
        .css_classes(["suggested-action", "pill"])
        .build();

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scrolled));
    view.add_bottom_bar(&got_it);

    let dialog = adw::Dialog::builder()
        .title(i18n("Help: %s").replace("%s", &help.title))
        .content_width(620)
        .content_height(700)
        .child(&view)
        .build();
    got_it.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));
    dialog.set_default_widget(Some(&got_it));
    dialog.present(Some(widget));
}

fn heading(text: &str) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["heading", "help-heading"])
        .build()
}

/// A numbered step: the number in a round badge, the instruction beside it.
fn step_row(n: u32, text: &str) -> gtk4::Box {
    let number = gtk4::Label::builder()
        .label(n.to_string())
        .valign(gtk4::Align::Start)
        .css_classes(["help-step-number"])
        .build();
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.append(&number);
    row.append(
        &gtk4::Label::builder()
            .label(text)
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    row
}

/// The help for a page.
// One entry per page; splitting the table would only scatter it.
#[allow(clippy::too_many_lines)]
fn content(tab: &str) -> Help {
    match tab {
        "home" => Help {
            icon: "go-home-symbolic",
            title: i18n("Home"),
            lead: i18n(
                "Where Turbo is switched on, and where you see at a glance what games are getting.",
            ),
            steps: vec![
                i18n("Choose what games should favour: one of the four presets under the power button."),
                i18n(
                    "Press the power button. The disc fills as each part of Turbo is really applied, and the caption under it says which.",
                ),
                i18n(
                    "Start a game from Steam, Lutris, Heroic, the application menu or Profiles. The card at the bottom shows it with its flags.",
                ),
                i18n("Press the power button again to switch Turbo off: everything it changed is put back."),
            ],
            parts: std::iter::once((
                "system-shutdown-symbolic",
                i18n("Turbo"),
                i18n(
                    "Off, BiGame-mode does not intervene in games. On, falcond applies each game's profile as it starts and undoes it when the game closes.",
                ),
            ))
            // The presets in the words and icons the picker itself uses.
            .chain(turbo_preset::ALL.into_iter().map(|p| {
                (
                    turbo_presets::icon(p),
                    i18n(p.label()),
                    i18n(p.description()),
                )
            }))
            .chain([
                (
                    "utilities-system-monitor-symbolic",
                    i18n("Live readings"),
                    i18n("CPU clock, GPU load and network latency, read while Home is on screen."),
                ),
                (
                    "input-gaming-symbolic",
                    i18n("The game card"),
                    i18n(
                        "While a game runs: which one, how it runs, and one coloured flag per feature (profile, scheduler, power, Gamescope, upscaling, frame generation, MangoHud, vkBasalt, frame cap). Green is in force, yellow needs a look, grey is off. Create profile appears when the game has none; View optimization details opens the full report.",
                    ),
                ),
            ])
            .collect(),
            tip: i18n(
                "A preset is chosen with Turbo off and ends when Turbo does. Steam opened before Turbo gets the preset when it is reopened, and Home offers to reopen it.",
            ),
        },
        "profiles" => Help {
            icon: "applications-games-symbolic",
            title: i18n("Profiles"),
            lead: i18n("Your installed games, and the profile that tunes each one."),
            steps: vec![
                i18n("Find the game: search by name, or filter by launcher and by whether it has a profile."),
                i18n(
                    "Open its menu (⋮) and choose Create with Wizard, or Edit when it already has a profile.",
                ),
                i18n(
                    "Change only what this game needs. Options left on “General configuration” follow Tuning.",
                ),
                i18n("Save, then start the game with Launch (Turbo) or from its own launcher."),
            ],
            parts: vec![
                (
                    "view-grid-symbolic",
                    i18n("Game library"),
                    i18n(
                        "Games from Steam, Lutris, Heroic and the application menu, native or Flatpak; only installed ones appear. A green dot is your own profile, blue one that ships with falcond.",
                    ),
                ),
                (
                    "document-edit-symbolic",
                    i18n("Profile editor"),
                    i18n(
                        "The same sections as Tuning (Performance, Display, Image quality, Frame generation, Monitoring, Advanced), for this game only. falcond applies it while the game runs.",
                    ),
                ),
                (
                    "applications-science-symbolic",
                    i18n("AI Graphics"),
                    i18n(
                        "Scans the game (graphics API, the upscalers it ships, anti-cheat, the GPU it renders on) and recommends a plan. Nothing changes until you click Apply, and Restore the game's graphics puts its files back.",
                    ),
                ),
                (
                    "utilities-system-monitor-symbolic",
                    i18n("Measure the difference"),
                    i18n(
                        "Runs the game with and without optimizations, alternating, and calls a change an improvement only when it beats the variation.",
                    ),
                ),
                (
                    "document-open-symbolic",
                    i18n("Import and export"),
                    i18n("Load a profile from a .conf or .toml file, or drop it on the list; export one to share it."),
                ),
            ],
            tip: i18n(
                "falcond recognises a game by the name of its process, as the system monitor shows it, not by its title. When a game without a profile starts with Turbo on, BiGame-mode can offer to create one.",
            ),
        },
        "tuning" => Help {
            icon: "preferences-system-symbolic",
            title: i18n("Tuning"),
            lead: i18n(
                "The general configuration: what every game gets, unless its own profile says otherwise.",
            ),
            steps: Vec::new(),
            parts: vec![
                (
                    "power-profile-performance-symbolic",
                    i18n("Performance"),
                    i18n(
                        "Performance mode, the sched-ext CPU scheduler for games without one of their own, and 3D V-Cache where the processor has it.",
                    ),
                ),
                (
                    "video-display-symbolic",
                    i18n("Display"),
                    i18n(
                        "Gamescope for games started from BiGame-mode: filter, sharpness, render and output sizes. Changing the sizes updates the Steam games that use it.",
                    ),
                ),
                (
                    "image-x-generic-symbolic",
                    i18n("Image quality"),
                    i18n("Wine FSR for Proton games in exclusive fullscreen, and vkBasalt's filters."),
                ),
                (
                    "media-skip-forward-symbolic",
                    i18n("Frame generation"),
                    i18n(
                        "lsfg-vk: the global switch, your Lossless.dll and each game's multiplier. It raises the frame rate you see, not the one the game renders.",
                    ),
                ),
                (
                    "utilities-system-monitor-symbolic",
                    i18n("Monitoring"),
                    i18n("The MangoHud overlay, with styles inspired by the Steam Deck."),
                ),
                (
                    "emblem-system-symbolic",
                    i18n("Advanced"),
                    i18n(
                        "falcond's own settings, the CPU governor, sched-ext availability, the Gamescope options this build accepts, and the environment file.",
                    ),
                ),
                (
                    "dialog-question-symbolic",
                    i18n("Conflicts are always asked"),
                    i18n(
                        "Switching on a technology that does the same job as one already on (Wine FSR, Gamescope's upscaling, OptiScaler, lsfg-vk) asks which to keep. Nothing is settled in silence.",
                    ),
                ),
                (
                    "action-unavailable-symbolic",
                    i18n("Only what works here"),
                    i18n(
                        "What this machine cannot do is shown as not supported or missing, with the command that fixes it, never as a control that does nothing.",
                    ),
                ),
            ],
            tip: i18n(
                "Every change is saved at once. Restore Defaults, in the main menu, puts this page back to the recommended values.",
            ),
        },
        "dashboard" => Help {
            icon: "speedometer-symbolic",
            title: i18n("Details"),
            lead: i18n("What is really in force for the game, and how BiGame-mode knows it."),
            steps: Vec::new(),
            parts: vec![
                (
                    "emblem-ok-symbolic",
                    i18n("Overview"),
                    i18n(
                        "One line on how the machine stands, and a chip per item: Turbo, falcond, the profile, power, the scheduler, the GPU, Gamescope, upscaling, frame generation.",
                    ),
                ),
                (
                    "video-display-symbolic",
                    i18n("Telemetry and graphics cards"),
                    i18n(
                        "Live CPU and GPU readings, and one card per GPU with its load, clock, video memory, temperature and power, and which one renders the game.",
                    ),
                ),
                (
                    "view-list-bullet-symbolic",
                    i18n("Performance and video pipeline"),
                    i18n(
                        "Each item is Active, Waiting for a game, Configured, Not detected, Off, Missing dependency or Not supported. Open a row for what that means, the evidence and the fix.",
                    ),
                ),
                (
                    "dialog-warning-symbolic",
                    i18n("Problems"),
                    i18n(
                        "Everything that needs attention, classed as fixable, needing you, hardware or information, with commands to copy. A hardware limit is never an error.",
                    ),
                ),
                (
                    "network-wireless-symbolic",
                    i18n("Network, background load and Steam"),
                    i18n(
                        "The connection and a DNS comparison, programs competing for the CPU, and Steam launch options that call a missing program.",
                    ),
                ),
                (
                    "edit-copy-symbolic",
                    i18n("Support report"),
                    i18n(
                        "Everything above in one text to paste into a forum or an issue, with your user name, home folder and host masked.",
                    ),
                ),
            ],
            tip: i18n(
                "Start a game and keep this page open: it is read only while it is on screen, so it costs nothing the rest of the time.",
            ),
        },
        "logs" => Help {
            icon: "utilities-terminal-symbolic",
            title: i18n("Logs"),
            lead: i18n("What matters in a game session, read from the system journal."),
            steps: Vec::new(),
            parts: vec![
                (
                    "view-list-bullet-symbolic",
                    i18n("One view"),
                    i18n(
                        "falcond, BiGame-mode, power-profiles-daemon, scx_loader, Gamescope and the kernel's GPU messages together.",
                    ),
                ),
                (
                    "system-search-symbolic",
                    i18n("Filter and search"),
                    i18n("Show only errors, warnings or one source, or search the text."),
                ),
                (
                    "media-playback-start-symbolic",
                    i18n("Follow"),
                    i18n("New entries appear while the page is open; nothing is read while it is not."),
                ),
                (
                    "document-save-symbolic",
                    i18n("Copy and export"),
                    i18n(
                        "Copy what is shown, or save it to a file with your user name, home folder and host masked.",
                    ),
                ),
            ],
            tip: i18n(
                "When something goes wrong in a game, open Logs right after it: the entries around that moment usually say why.",
            ),
        },
        "settings" => Help {
            icon: "emblem-system-symbolic",
            title: i18n("Settings"),
            lead: i18n("How BiGame-mode looks and behaves, and how to hand falcond back."),
            steps: Vec::new(),
            parts: vec![
                (
                    "preferences-desktop-appearance-symbolic",
                    i18n("Appearance"),
                    i18n("The Default or Gamer theme, light, dark or the desktop's. Only the look changes."),
                ),
                (
                    "system-run-symbolic",
                    i18n("Start in the background at login"),
                    i18n(
                        "In the tray, so games started from Steam are noticed even with the window closed.",
                    ),
                ),
                (
                    "applications-games-symbolic",
                    i18n("Game profiles"),
                    i18n("Whether a profile is offered when a game without one starts with Turbo on."),
                ),
                (
                    "network-wireless-symbolic",
                    i18n("Notifications and ping target"),
                    i18n(
                        "Notifications when a game starts or exits, and the address the Details page pings for its latency graph.",
                    ),
                ),
                (
                    "emblem-synchronizing-symbolic",
                    i18n("Profiles from an older BiGame-mode"),
                    i18n("Profiles that can never match their game, and the fix."),
                ),
                (
                    "edit-undo-symbolic",
                    i18n("Hand falcond back"),
                    i18n(
                        "Return falcond to exactly the state it was in before BiGame-mode first changed it.",
                    ),
                ),
            ],
            tip: i18n("The welcome screen can be opened again from the main menu, under Welcome."),
        },
        "report" => Help {
            icon: "view-list-bullet-symbolic",
            title: i18n("Optimization Report"),
            lead: i18n("What the last switch of Turbo did, item by item, and why."),
            steps: Vec::new(),
            parts: vec![
                (
                    "object-select-symbolic",
                    i18n("Applied and verified"),
                    i18n("Changed, then read back from the system to confirm it took."),
                ),
                (
                    "system-users-symbolic",
                    i18n("Managed per game"),
                    i18n("Left to the component that owns it, which sets it for each game."),
                ),
                (
                    "action-unavailable-symbolic",
                    i18n("Skipped, not available, conflicts avoided"),
                    i18n(
                        "Deliberately left alone, not possible on this machine, or a second controller of the same thing that was not used.",
                    ),
                ),
                (
                    "dialog-warning-symbolic",
                    i18n("Did not take effect"),
                    i18n("Attempted, and the system did not change: the reason is on the row."),
                ),
            ],
            tip: i18n("The back button at the top returns to Home."),
        },
        _ => Help {
            icon: "help-about-symbolic",
            title: i18n("Help"),
            lead: i18n("No help available for this view."),
            steps: Vec::new(),
            parts: Vec::new(),
            tip: i18n("The welcome screen can be opened again from the main menu, under Welcome."),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_help_of_its_own() {
        for page in crate::window::PAGES.iter().copied().chain(["report"]) {
            let help = content(page);
            assert!(!help.parts.is_empty(), "{page} has no help");
            assert!(!help.lead.is_empty() && !help.tip.is_empty(), "{page}");
        }
    }
}
