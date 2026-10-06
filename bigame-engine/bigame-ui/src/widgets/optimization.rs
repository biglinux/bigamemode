//! The settings Tuning, a game's profile and the wizard share.
//!
//! One vocabulary for three scopes. Tuning shows the *general*
//! configuration; a profile shows one game's (`widgets::game_fields`), where
//! every value that can be left to the general one says so ("General
//! configuration", and what that is now) instead of showing `none`; the
//! wizard places the very same fields one per step. The rules behind them —
//! the fixed choices, where an effective value comes from, which
//! technologies collide — are `bigame_core::optimization`'s; nothing here
//! decides one on its own.
//!
//! What the machine cannot do is a *missing* or *not supported* row with the
//! reason, never a control that would do nothing.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::capabilities::Support;
use bigame_core::config::FalcondConfig;
use bigame_core::optimization::{
    self as opt, Feature, GameOptimization, SCHEDULER_MODES, VCACHE_MODES,
};
use bigame_core::overview::State;
use bigame_core::video_config::VideoConfig;

use crate::i18n::i18n;
use crate::widgets::status::Chip;

// ── The machine ─────────────────────────────────────────────────────────────

/// What the machine has, read once for a page: file reads, no process.
// Independent facts about the machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Machine {
    /// falcond's general configuration.
    pub general: FalcondConfig,
    /// The general launch settings.
    pub video: VideoConfig,
    /// `none`, then the schedulers installed that falcond knows.
    pub schedulers: Vec<String>,
    /// Whether falcond can switch schedulers here, and why not.
    pub sched: Support,
    /// Scheduler binaries found.
    pub sched_installed: Vec<String>,
    /// A dual-CCD X3D processor with the `amd_x3d_vcache` driver.
    pub vcache: bool,
    /// Gamescope is on `PATH`.
    pub gamescope: bool,
    /// The lsfg-vk layer is installed.
    pub lsfg: bool,
    /// Its `Lossless.dll` is configured and exists.
    pub lsfg_dll: bool,
    /// `mangohud` is on `PATH`.
    pub mangohud: bool,
}

impl Machine {
    /// Read the machine.
    #[must_use]
    pub fn detect() -> Self {
        let caps = bigame_core::capabilities::SchedExtCaps::detect();
        Self {
            general: bigame_core::config::read().unwrap_or_default(),
            video: bigame_core::video_config::load(),
            schedulers: bigame_core::sched::detect_installed(),
            sched: caps.switchable(),
            sched_installed: caps.installed.clone(),
            vcache: bigame_core::vcache::is_available(),
            gamescope: bigame_core::capabilities::which("gamescope").is_some(),
            lsfg: bigame_core::fg::layer_installed(),
            lsfg_dll: bigame_core::fg::is_lossless_dll_ready(),
            mangohud: bigame_core::capabilities::which("mangohud").is_some(),
        }
    }
}

/// How a game's own launch settings (Gamescope, Wine FSR, vkBasalt) reach
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reach {
    /// The Steam client starts it: its launch options, written with Steam
    /// closed.
    Steam,
    /// Heroic starts it: its settings there, written with Heroic closed.
    Heroic {
        /// Heroic's Flatpak, which needs Flathub's extensions for Gamescope
        /// and vkBasalt.
        flatpak: bool,
    },
    /// BiGame-mode starts it (Profiles → Launch).
    Launch,
    /// Not known here (a profile with no game card, the wizard): whichever
    /// of them starts it.
    #[default]
    Unknown,
    /// Its own launcher starts it, and BiGame-mode cannot.
    Nothing,
}

impl Reach {
    /// Whether its launcher's own settings carry the game's launch
    /// settings (Steam, Heroic), where Automatic runs Gamescope only for the
    /// game's own values rather than following Tuning's switch.
    #[must_use]
    pub fn launcher_written(self) -> bool {
        matches!(self, Self::Steam | Self::Heroic { .. })
    }

    /// Heroic's Flatpak starts it.
    #[must_use]
    pub fn heroic_flatpak(self) -> bool {
        self == Self::Heroic { flatpak: true }
    }
}

/// The game a profile is for, as far as its pages need it.
#[derive(Debug, Clone, Default)]
pub struct Game {
    /// Its title, for headings.
    pub title: String,
    /// Where AI Graphics works on it, when its install folder is known.
    pub target: Option<bigame_core::graphics::Target>,
    /// What `OptiScaler` does in it now.
    pub optiscaler: Vec<Feature>,
    /// How its own launch settings reach it.
    pub reach: Reach,
}

impl Game {
    /// Read what `OptiScaler` does in the game whose process is `process`.
    #[must_use]
    pub fn detect(
        process: &str,
        title: &str,
        target: Option<bigame_core::graphics::Target>,
    ) -> Self {
        Self {
            title: if title.is_empty() {
                process.to_owned()
            } else {
                title.to_owned()
            },
            target,
            optiscaler: if process.is_empty() {
                Vec::new()
            } else {
                opt::optiscaler_features(process)
            },
            reach: Reach::Unknown,
        }
    }
}

// ── Page structure ──────────────────────────────────────────────────────────

/// Which settings a page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    /// Tuning: what every game gets.
    General,
    /// One game's profile.
    Game(&'a str),
}

/// The card at the top of Tuning and of a profile that says whose settings
/// these are, in the same words on both pages.
#[must_use]
pub fn scope_banner(scope: Scope<'_>) -> adw::PreferencesGroup {
    let (icon, title, text) = match scope {
        Scope::General => (
            "applications-system-symbolic",
            i18n("General configuration · Global profile"),
            i18n(
                "A game without a profile of its own gets exactly these settings: this page is the Global profile, used by every game marked “Without a profile”. A game's profile replaces only what it sets; everything else still follows this page.",
            ),
        ),
        Scope::Game(game) => (
            "input-gaming-symbolic",
            i18n("Profile for %s").replace("%s", game),
            i18n("Only for this game. Options left on “General configuration” follow Tuning."),
        ),
    };
    let image = gtk4::Image::from_icon_name(icon);
    image.set_pixel_size(32);
    image.set_valign(gtk4::Align::Center);
    image.add_css_class("scope-icon");
    let heading = gtk4::Label::builder()
        .label(&title)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["title-4"])
        .build();
    let body = gtk4::Label::builder()
        .label(&text)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let words = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.append(&heading);
    words.append(&body);
    let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
    card.add_css_class("scope-card");
    card.append(&image);
    card.append(&words);
    let group = adw::PreferencesGroup::new();
    group.add(&card);
    group
}

/// A titled section. Every page uses the same six, in the same order:
/// Performance, Display, Image quality, Frame generation, Monitoring,
/// Advanced.
#[must_use]
pub fn section(title: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(title);
    group
}

/// A row that says something is not there, with the command that installs
/// it — in place of a control that could not work.
#[must_use]
pub fn missing_row(title: &str, what: &str, command: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(format!("{what}\n→ {command}"))
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    let chip = Chip::new(State::Missing);
    row.add_suffix(chip.widget());
    let copy = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(i18n("Copy the command"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    copy.update_property(&[gtk4::accessible::Property::Label(&i18n("Copy the command"))]);
    let command = command.to_owned();
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&command);
        crate::widgets::toast::show(b, &i18n("Copied"));
    });
    row.add_suffix(&copy);
    row
}

/// A row that says the hardware cannot do something — a fact, not a fault.
#[must_use]
pub fn unsupported_row(title: &str, why: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(why)
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    let chip = Chip::new(State::Unsupported);
    row.add_suffix(chip.widget());
    row
}

/// The row for a scheduler falcond cannot switch to here, and why; `None`
/// when it can.
#[must_use]
pub fn scheduler_unavailable_row(m: &Machine) -> Option<adw::ActionRow> {
    let title = i18n("CPU scheduler (sched-ext)");
    Some(match &m.sched {
        Support::Available => return None,
        Support::Unsupported(_) => unsupported_row(
            &title,
            &i18n(
                "The running kernel was built without sched_ext. A kernel with it (BigLinux's default) is needed.",
            ),
        ),
        Support::NotInstalled(pkg) if pkg == "scx-tools" => missing_row(
            &title,
            &i18n(
                "Schedulers are installed, but scx_loader is not; falcond switches schedulers only through it.",
            ),
            "sudo pacman -S scx-tools && sudo systemctl enable --now scx_loader",
        ),
        Support::NotInstalled(_) => missing_row(
            &title,
            &i18n("No sched-ext scheduler is installed. Game profiles can then ask for one."),
            "sudo pacman -S scx-scheds scx-tools",
        ),
        Support::ServiceDown(_) => missing_row(
            &title,
            &i18n("scx_loader is installed but its service is not running."),
            "sudo systemctl enable --now scx_loader",
        ),
    })
}

/// The row for 3D V-Cache on a processor without it.
#[must_use]
pub fn vcache_unsupported_row() -> adw::ActionRow {
    let row = unsupported_row(
        &i18n("3D V-Cache"),
        &i18n(
            "Not supported on this processor (only AMD X3D processors with two CCDs have a choice).",
        ),
    );
    row.add_suffix(&crate::widgets::info::vcache_button());
    row
}

// ── Choices ─────────────────────────────────────────────────────────────────

/// A combo row over fixed values, read and set by value rather than index.
#[derive(Clone)]
pub struct Picker {
    /// The row.
    pub row: adw::ComboRow,
    model: gtk4::StringList,
    ids: Rc<RefCell<Vec<String>>>,
}

impl Picker {
    /// A row titled `title` over `(value, label)` pairs, showing `selected`.
    #[must_use]
    pub fn new(title: &str, subtitle: &str, items: &[(String, String)], selected: &str) -> Self {
        let labels: Vec<&str> = items.iter().map(|(_, l)| l.as_str()).collect();
        let model = gtk4::StringList::new(&labels);
        let row = adw::ComboRow::builder()
            .title(title)
            .model(&model)
            .use_subtitle(false)
            .build();
        if !subtitle.is_empty() {
            row.set_subtitle(subtitle);
        }
        // A long subtitle wraps, but it asks for its whole width first and
        // squeezes the chosen value to an ellipsis in a narrow window or a
        // longer language: its natural width is capped so the value keeps
        // room.
        cap_subtitle(row.upcast_ref(), 34);
        keep_value_width(&row);
        let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
        let me = Self {
            row,
            model,
            ids: Rc::new(RefCell::new(ids)),
        };
        me.set(selected);
        me
    }

    /// The selected value.
    #[must_use]
    pub fn value(&self) -> String {
        self.ids
            .borrow()
            .get(self.row.selected() as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Select `value`. A saved value the list does not offer (a scheduler
    /// since uninstalled) is shown as it is, marked, rather than as the
    /// first item while the configuration still holds it; an empty one
    /// selects the first item.
    pub fn set(&self, value: &str) {
        let found = self.ids.borrow().iter().position(|id| id == value);
        let i = match found {
            Some(i) => i,
            None if value.is_empty() => 0,
            None => {
                self.model
                    .append(&i18n("%s (not available)").replace("%s", value));
                let mut ids = self.ids.borrow_mut();
                ids.push(value.to_owned());
                ids.len() - 1
            }
        };
        self.row.set_selected(u32::try_from(i).unwrap_or(0));
    }

    /// Call `f` with the new value whenever the selection changes.
    pub fn connect_changed(&self, f: impl Fn(&str) + 'static) {
        let ids = Rc::clone(&self.ids);
        self.row.connect_selected_notify(move |r| {
            let id = ids.borrow().get(r.selected() as usize).cloned();
            if let Some(id) = id {
                f(&id);
            }
        });
    }
}

/// Cap the natural width of a row's subtitle at `chars` (it still wraps).
pub fn cap_subtitle(widget: &gtk4::Widget, chars: i32) {
    if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
        if label.has_css_class("subtitle") {
            label.set_max_width_chars(chars);
        }
        return;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        cap_subtitle(&c, chars);
        child = c.next_sibling();
    }
}

/// Keep room for a combo row's value. GTK lays a row out for its height, so
/// a subtitle that just fits on one line claims the width first and squeezes
/// the value to an ellipsis ("Automá…"); the value keeps the width of the
/// chosen item instead (at most 260 px: a longer one still ellipsizes).
pub fn keep_value_width(row: &adw::ComboRow) {
    fn value_view(w: &gtk4::Widget) -> Option<gtk4::Widget> {
        if w.is::<gtk4::ListView>() && w.has_css_class("inline") {
            return Some(w.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if let Some(found) = value_view(&c) {
                return Some(found);
            }
            child = c.next_sibling();
        }
        None
    }
    let Some(view) = value_view(row.upcast_ref()) else {
        return;
    };
    let fit = move |row: &adw::ComboRow| {
        let text = row
            .model()
            .and_then(|m| m.downcast::<gtk4::StringList>().ok())
            .and_then(|m| m.string(row.selected()));
        let width = text.map_or(0, |t| row.create_pango_layout(Some(&t)).pixel_size().0);
        // The item's own padding on top of the text.
        view.set_width_request((width + 14).min(260));
    };
    fit(row);
    row.connect_selected_notify(fit);
}

/// `scx_lavd`, or the kernel's own for `none`.
#[must_use]
pub fn scheduler_name(id: &str) -> String {
    if opt::inherits(id) {
        i18n("Kernel default")
    } else {
        format!("scx_{id}")
    }
}

/// A mode's name.
#[must_use]
pub fn mode_name(id: &str) -> String {
    opt::find(SCHEDULER_MODES, id).map_or_else(|| id.to_owned(), |c| i18n(c.label))
}

/// A 3D V-Cache mode's name.
#[must_use]
pub fn vcache_name(id: &str) -> String {
    opt::find(VCACHE_MODES, if id.is_empty() { "none" } else { id })
        .map_or_else(|| id.to_owned(), |c| i18n(c.label))
}

/// `scx_lavd · Gaming`, or the kernel's own.
#[must_use]
pub fn scheduler_summary(sched: &str, mode: &str) -> String {
    if opt::inherits(sched) {
        scheduler_name(sched)
    } else {
        format!("{} · {}", scheduler_name(sched), mode_name(mode))
    }
}

/// `General configuration (…)`: the choice that leaves a value to Tuning,
/// naming what that is now.
#[must_use]
pub fn inherit_label(what: &str) -> String {
    i18n("General configuration (%s)").replace("%s", what)
}

/// The scheduler choices: for a game, the first keeps the general one.
#[must_use]
pub fn scheduler_items(m: &Machine, scope: Scope<'_>) -> Vec<(String, String)> {
    m.schedulers
        .iter()
        .map(|id| {
            let label = if opt::inherits(id) {
                match scope {
                    Scope::General => scheduler_name(id),
                    // The value it stands for is in the row's subtitle, so
                    // the choice stays short enough to read.
                    Scope::Game(_) => i18n("General configuration"),
                }
            } else {
                scheduler_name(id)
            };
            (id.clone(), label)
        })
        .collect()
}

/// The mode choices.
#[must_use]
pub fn mode_items() -> Vec<(String, String)> {
    SCHEDULER_MODES
        .iter()
        .map(|c| (c.id.to_owned(), i18n(c.label)))
        .collect()
}

/// The 3D V-Cache choices: for a game, the first keeps the general one.
#[must_use]
pub fn vcache_items(scope: Scope<'_>) -> Vec<(String, String)> {
    VCACHE_MODES
        .iter()
        .map(|c| {
            let label = match (c.id, scope) {
                ("none", Scope::Game(_)) => i18n("General configuration"),
                _ => i18n(c.label),
            };
            (c.id.to_owned(), label)
        })
        .collect()
}

/// What a profile holds, as label/value lines: the wizard's review and
/// anything else that summarises a profile. Unavailable features say so
/// rather than disappearing.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn summary(g: &GameOptimization, m: &Machine) -> Vec<(String, String)> {
    use bigame_core::gamescope::Mode;
    let yes_no = |b: bool| if b { i18n("On") } else { i18n("Off") };
    let mut rows = vec![(i18n("Program"), g.profile.name.clone())];
    rows.push((
        i18n("Performance mode"),
        match opt::performance(&g.profile, &m.general) {
            opt::Performance::On => i18n("On"),
            opt::Performance::Off => i18n("Off"),
            opt::Performance::BlockedByGeneral => i18n("On, but off in Tuning for every game"),
        },
    ));
    rows.push((
        i18n("Keep the screen awake"),
        yes_no(g.profile.idle_inhibit),
    ));
    rows.push((
        i18n("CPU scheduler"),
        if m.sched.is_available() {
            let e = opt::effective_scheduler(Some(&g.profile), &m.general);
            match e.source {
                opt::Source::Game => scheduler_summary(&e.value, &e.mode),
                _ => inherit_label(&scheduler_summary(
                    &m.general.scx_sched,
                    &m.general.scx_sched_props,
                )),
            }
        } else {
            i18n("Not available on this system")
        },
    ));
    rows.push((
        i18n("3D V-Cache"),
        if m.vcache {
            if opt::inherits(&g.profile.vcache_mode) {
                inherit_label(&vcache_name(&m.general.vcache_mode))
            } else {
                vcache_name(&g.profile.vcache_mode)
            }
        } else {
            i18n("Not supported on this processor")
        },
    ));
    rows.push((
        "Gamescope".to_owned(),
        if m.gamescope {
            let mode = match g.profile.gamescope_mode {
                Mode::Auto => inherit_label(&if m.video.upscaling.gamescope_enabled {
                    i18n("On")
                } else {
                    i18n("Off")
                }),
                Mode::Enabled => i18n("Always"),
                Mode::Disabled => i18n("Never"),
            };
            match g.launch.render {
                Some((w, h)) if g.profile.gamescope_mode != Mode::Disabled && w > 0 && h > 0 => {
                    format!("{mode} · {w} × {h}")
                }
                _ => mode,
            }
        } else {
            i18n("Not installed")
        },
    ));
    let own_or_general = |own: Option<bool>, general: bool| match own {
        Some(v) => yes_no(v),
        None => inherit_label(&yes_no(general)),
    };
    rows.push((
        "Wine FSR".to_owned(),
        own_or_general(g.launch.wine_fsr, m.video.upscaling.wine_fsr_enabled),
    ));
    if bigame_core::capabilities::vkbasalt_installed() {
        rows.push((
            "vkBasalt".to_owned(),
            own_or_general(g.launch.vkbasalt, m.video.upscaling.vkbasalt_enabled),
        ));
    }
    rows.push((
        i18n("Frame generation"),
        if !m.lsfg {
            i18n("lsfg-vk is not installed")
        } else if g.frame_generation.on() {
            format!("lsfg-vk · {}×", g.frame_generation.multiplier)
        } else {
            i18n("Off")
        },
    ));
    rows.push((
        "MangoHud".to_owned(),
        if m.mangohud {
            match g.mangohud {
                bigame_core::mangohud::Mode::Off => i18n("Off"),
                bigame_core::mangohud::Mode::On => i18n("On"),
                bigame_core::mangohud::Mode::Forced => i18n("Forced"),
            }
        } else {
            i18n("Not installed")
        },
    ));
    rows
}

/// Say what a save did, in one toast; a part that failed is named with its
/// reason, never hidden behind "Profile saved".
pub fn report_save(anchor: &impl IsA<gtk4::Widget>, report: &opt::SaveReport) {
    use bigame_core::mangohud::Applied;
    let mut problems = Vec::new();
    if let Some(e) = &report.frame_generation {
        problems.push(format!("lsfg-vk: {}", crate::i18n::error_text(e)));
    }
    if let Some(e) = &report.launch {
        problems.push(format!(
            "{}: {}",
            i18n("Launch settings"),
            crate::i18n::error_text(e)
        ));
    }
    let mut note = None;
    if let Some(g) = &report.gamescope {
        use bigame_core::steam_gamescope::Applied as G;
        match g {
            Ok(G::SteamRunning) => problems.push(i18n(
                "Launch options: close Steam first: it keeps them in memory and would overwrite the change.",
            )),
            Ok(G::Written(opts)) if !opts.is_empty() => {
                note = Some(i18n("Steam launch options: %s").replace("%s", opts));
            }
            // Never one Gamescope inside another.
            Ok(G::TheirGamescope(opts)) => {
                note = Some(
                    i18n("Steam launch options: %s. They already run a Gamescope of your own, so Big Game Mode's was not added.")
                        .replace("%s", opts),
                );
            }
            Ok(_) => {}
            Err(e) => problems.push(format!(
                "{}: {}",
                i18n("Launch options"),
                crate::i18n::error_text(e)
            )),
        }
    }
    let close_heroic = report
        .heroic
        .as_ref()
        .and_then(|h| heroic_outcome(h, &mut problems, &mut note));
    if let Some(m) = &report.mangohud {
        match m {
            Ok(Applied::SteamRunning) => problems.push(i18n(
                "MangoHud: close Steam first: it keeps its launch options in memory and would overwrite the change.",
            )),
            Ok(Applied::LauncherRunning(name)) => problems.push(
                i18n("MangoHud: close %s first: it keeps the game's settings in memory and would overwrite the change.")
                    .replace("%s", name),
            ),
            Ok(Applied::Launcher {
                name,
                missing_extension: Some(command),
            }) => {
                note = Some(
                    i18n("Written into the game's settings in %l, but %l is a Flatpak without MangoHud's Flatpak extension, so the overlay cannot load. Install it and restart %l: %c")
                        .replace("%l", name)
                        .replace("%c", command),
                );
            }
            Ok(_) => {}
            Err(e) => problems.push(format!("MangoHud: {}", crate::i18n::error_text(e))),
        }
    }
    if problems.is_empty() {
        crate::widgets::toast::show(anchor, &note.unwrap_or_else(|| i18n("Profile saved")));
    } else {
        crate::widgets::toast::error(
            anchor,
            &i18n("Profile saved, but not every part was applied"),
            &problems.join("\n"),
        );
    }
    if let Some(launcher) = close_heroic {
        let process = report.process.clone();
        offer_close_heroic(
            anchor,
            launcher,
            &i18n(
                "Heroic is open: this game's launch settings were not written into its settings there",
            ),
            move || bigame_core::optimization::apply_heroic(&process).map(|_| ()),
        );
    }
}

/// What writing a Heroic game's settings did, as a save's problem or note;
/// the Heroic to offer to close, when it is open and runs no game.
fn heroic_outcome(
    applied: &anyhow::Result<bigame_core::heroic_launch::Applied>,
    problems: &mut Vec<String>,
    note: &mut Option<String>,
) -> Option<bigame_core::launchers::Launcher> {
    use bigame_core::heroic_launch::Applied as H;
    match applied {
        Ok(H::Written) => {
            note.get_or_insert_with(|| {
                i18n("Profile saved. Written into Heroic's settings for this game.")
            });
        }
        Ok(H::HeroicRunning {
            game_running: true, ..
        }) => problems.push(i18n(
            "Launch settings: Heroic is running a game. Close the game, then save again with Heroic closed: it keeps this game's settings in memory and would overwrite the change.",
        )),
        Ok(H::HeroicRunning { launcher, .. }) => {
            problems.push(i18n(
                "Launch settings: close Heroic first: it keeps this game's settings in memory and would overwrite the change.",
            ));
            return Some(*launcher);
        }
        Ok(_) => {}
        Err(e) => problems.push(format!(
            "{}: {}",
            i18n("Heroic's settings"),
            crate::i18n::error_text(e)
        )),
    }
    None
}

/// Offer to close `launcher` — a Heroic that runs no game, as it said when
/// the change was refused — run `job` while it is closed, and open it again.
/// Closing is refused again, and `job` does not run, if a game from it has
/// started since.
pub fn offer_close_heroic(
    anchor: &impl IsA<gtk4::Widget>,
    launcher: bigame_core::launchers::Launcher,
    message: &str,
    job: impl Fn() -> anyhow::Result<()> + Send + Clone + 'static,
) {
    let a = anchor.clone().upcast::<gtk4::Widget>();
    crate::widgets::toast::with_action(
        anchor,
        message,
        &i18n("Close Heroic and apply"),
        move || {
            let (a, job) = (a.clone(), job.clone());
            glib::spawn_future_local(async move {
                let done = gio::spawn_blocking(move || {
                    bigame_core::launchers::while_closed(launcher, job)
                })
                .await;
                match done {
                    Ok(Ok(Ok(()))) => crate::widgets::toast::show(
                        &a,
                        &i18n("Written into Heroic's settings; Heroic was opened again"),
                    ),
                    Ok(Ok(Err(e))) => crate::widgets::toast::error(
                        &a,
                        &i18n("Heroic's settings were not written"),
                        &crate::i18n::error_text(&e),
                    ),
                    Ok(Err(e)) => crate::widgets::toast::error(
                        &a,
                        &i18n("Heroic was not closed, or could not be opened again"),
                        &crate::i18n::error_text(&e),
                    ),
                    Err(_) => {}
                }
            });
        },
    );
}
