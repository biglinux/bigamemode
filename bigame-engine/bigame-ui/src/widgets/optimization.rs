//! The settings Tuning, a game's profile and the wizard share.
//!
//! One vocabulary for three scopes. Tuning shows the *general*
//! configuration; a profile shows one game's, where every value that can be
//! left to the general one says so ("General configuration (`scx_lavd` ·
//! Gaming)") instead of showing `none`; the wizard places the very same
//! fields one per step. The rules behind them — the fixed choices, where an
//! effective value comes from, which technologies collide — are
//! `bigame_core::optimization`'s; nothing here decides one on its own.
//!
//! What the machine cannot do is a *missing* or *not supported* row with the
//! reason, never a control that would do nothing.

use std::cell::{Cell, RefCell};
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

use crate::i18n::{i18n, tr};
use crate::widgets::notice::{self, Kind, Notice};
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

/// The game a profile is for, as far as its pages need it.
#[derive(Debug, Clone, Default)]
pub struct Game {
    /// Its title, for headings.
    pub title: String,
    /// Where AI Graphics works on it, when its install folder is known.
    pub target: Option<bigame_core::graphics::Target>,
    /// What `OptiScaler` does in it now.
    pub optiscaler: Vec<Feature>,
    /// The Steam client starts it (its launch options are what reach it).
    pub steam: bool,
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
            steam: false,
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
            i18n("General configuration"),
            i18n(
                "Used by every game. A game's profile replaces what it sets for that game; everything else follows this page.",
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
    ids: Rc<Vec<String>>,
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
            ids: Rc::new(ids),
        };
        me.set(selected);
        me
    }

    /// The selected value.
    #[must_use]
    pub fn value(&self) -> String {
        self.ids
            .get(self.row.selected() as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Select `value` (the first item when it is not there).
    pub fn set(&self, value: &str) {
        let i = self
            .ids
            .iter()
            .position(|id| id == value)
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(0);
        self.row.set_selected(i);
    }

    /// Call `f` with the new value whenever the selection changes.
    pub fn connect_changed(&self, f: impl Fn(&str) + 'static) {
        let ids = Rc::clone(&self.ids);
        self.row.connect_selected_notify(move |r| {
            if let Some(id) = ids.get(r.selected() as usize) {
                f(id);
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
/// chosen item instead (at most 240 px: a longer one still ellipsizes).
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
        view.set_width_request((width + 6).min(240));
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

/// A chip saying whether a switch-like setting is on — so "is it on?" is
/// answered by a word, not only by a switch's position.
#[must_use]
pub fn on_off_chip(on: bool) -> Chip {
    let chip = Chip::new(if on { State::Configured } else { State::Off });
    chip.set(
        if on { State::Configured } else { State::Off },
        Some(&if on { i18n("On") } else { i18n("Off") }),
    );
    chip
}

// ── One game's fields ───────────────────────────────────────────────────────
//
// Each block builds its rows from a `GameOptimization` and writes them back
// into one. The profile editor shows them all on one page; the wizard shows
// them one per step. Same rows, same values, same object.

/// Performance mode and idle inhibit.
pub struct PerformanceFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    perf: adw::SwitchRow,
    idle: adw::SwitchRow,
}

impl PerformanceFields {
    /// Build from `g`.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, title: Option<&str>) -> Self {
        let group = adw::PreferencesGroup::new();
        if let Some(t) = title {
            group.set_title(t);
        }
        let perf = adw::SwitchRow::builder()
            .title(i18n("Performance mode"))
            .subtitle(i18n("The performance power profile while this game runs"))
            .active(g.profile.performance_mode)
            .build();
        group.add(&perf);
        // The general switch is a master switch in falcond: off, no game
        // gets the performance profile, whatever its profile says.
        let blocked = Notice::new(
            Kind::Warning,
            &i18n("Performance mode is off in Tuning"),
            &i18n(
                "falcond then gives no game the performance profile. Turn it on in Tuning → Performance.",
            ),
        );
        let general_on = m.general.enable_performance_mode;
        blocked.set_visible(g.profile.performance_mode && !general_on);
        group.add(blocked.widget());
        {
            let blocked = blocked.clone();
            perf.connect_active_notify(move |r| blocked.set_visible(r.is_active() && !general_on));
        }
        let idle = adw::SwitchRow::builder()
            .title(i18n("Keep the screen awake"))
            .subtitle(i18n("No screen saver or screen sleep while this game runs"))
            .active(g.profile.idle_inhibit)
            .build();
        group.add(&idle);
        Self { group, perf, idle }
    }

    /// Write into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        g.profile.performance_mode = self.perf.is_active();
        g.profile.idle_inhibit = self.idle.is_active();
    }
}

/// The scheduler and its mode, or why there is none to pick.
pub struct SchedulerFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    pickers: Option<(Picker, Picker)>,
}

impl SchedulerFields {
    /// Build from `g`.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, title: Option<&str>) -> Self {
        let group = adw::PreferencesGroup::new();
        if let Some(t) = title {
            group.set_title(t);
        }
        if let Some(row) = scheduler_unavailable_row(m) {
            group.add(&row);
            return Self {
                group,
                pickers: None,
            };
        }
        let scope = Scope::Game("");
        let sched = Picker::new(
            &i18n("CPU scheduler"),
            "",
            &scheduler_items(m, scope),
            if opt::inherits(&g.profile.scx_sched) {
                "none"
            } else {
                &g.profile.scx_sched
            },
        );
        sched
            .row
            .add_suffix(&crate::widgets::scheduler_info::button());
        group.add(&sched.row);
        let mode = Picker::new(
            &i18n("Scheduler mode"),
            "",
            &mode_items(),
            &g.profile.scx_sched_props,
        );
        mode.row
            .add_suffix(&crate::widgets::info::scheduler_modes_button());
        group.add(&mode.row);
        let sync = {
            let (sched_row, mode) = (sched.row.clone(), mode.clone());
            let general_sched = m.general.scx_sched.clone();
            let general_mode = m.general.scx_sched_props.clone();
            let own_mode = Rc::new(RefCell::new(g.profile.scx_sched_props.clone()));
            let was_inherit = Cell::new(opt::inherits(&g.profile.scx_sched));
            move |sched: &str| {
                let inherit = opt::inherits(sched);
                // Inheriting, the mode row shows the general mode it runs
                // with; choosing a scheduler brings back the game's own.
                if inherit && !was_inherit.get() {
                    *own_mode.borrow_mut() = mode.value();
                }
                if inherit {
                    mode.set(&general_mode);
                } else if was_inherit.get() {
                    mode.set(&own_mode.borrow());
                }
                was_inherit.set(inherit);
                mode.row.set_sensitive(!inherit);
                sched_row.set_subtitle(&if inherit {
                    i18n("Follows Tuning: %s")
                        .replace("%s", &scheduler_summary(&general_sched, &general_mode))
                } else {
                    String::new()
                });
                mode.row.set_subtitle(&if inherit {
                    if opt::inherits(&general_sched) {
                        i18n("No scheduler: the mode does nothing")
                    } else {
                        i18n("Follows Tuning")
                    }
                } else {
                    String::new()
                });
            }
        };
        sync(&sched.value());
        sched.connect_changed(sync);
        Self {
            group,
            pickers: Some((sched, mode)),
        }
    }

    /// Write into `g`. A value left to the general configuration is `none`,
    /// falcond's own way of saying "keep what is loaded", with the default
    /// mode.
    pub fn apply(&self, g: &mut GameOptimization) {
        let Some((sched, mode)) = &self.pickers else {
            return;
        };
        let s = sched.value();
        if opt::inherits(&s) {
            "none".clone_into(&mut g.profile.scx_sched);
            "default".clone_into(&mut g.profile.scx_sched_props);
        } else {
            g.profile.scx_sched = s;
            g.profile.scx_sched_props = mode.value();
        }
    }

    /// Whether the machine offers a choice.
    #[must_use]
    pub fn available(&self) -> bool {
        self.pickers.is_some()
    }
}

/// 3D V-Cache, or the fact that this processor has no choice.
pub struct VCacheFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    picker: Option<Picker>,
}

impl VCacheFields {
    /// Build from `g`.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, title: Option<&str>) -> Self {
        let group = adw::PreferencesGroup::new();
        if let Some(t) = title {
            group.set_title(t);
        }
        if !m.vcache {
            // The profile keeps its value: saving does not change it.
            group.add(&vcache_unsupported_row());
            return Self {
                group,
                picker: None,
            };
        }
        let picker = Picker::new(
            &i18n("3D V-Cache"),
            &i18n("Which CCD this game prefers"),
            &vcache_items(Scope::Game("")),
            &g.profile.vcache_mode,
        );
        picker
            .row
            .add_suffix(&crate::widgets::info::vcache_button());
        group.add(&picker.row);
        {
            let (row, general) = (picker.row.clone(), vcache_name(&m.general.vcache_mode));
            let sync = move |v: &str| {
                row.set_subtitle(&if opt::inherits(v) {
                    i18n("Follows Tuning: %s").replace("%s", &general)
                } else {
                    i18n("Which CCD this game prefers")
                });
            };
            sync(&picker.value());
            picker.connect_changed(sync);
        }
        Self {
            group,
            picker: Some(picker),
        }
    }

    /// Write into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        if let Some(p) = &self.picker {
            g.profile.vcache_mode = p.value();
        }
    }

    /// Whether the machine offers a choice.
    #[must_use]
    pub fn available(&self) -> bool {
        self.picker.is_some()
    }
}

/// Gamescope for one game: when it runs, and optionally its own sizes.
pub struct GamescopeFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    rows: Option<GamescopeRows>,
}

struct GamescopeRows {
    mode: adw::ComboRow,
    own: adw::ExpanderRow,
    width: adw::SpinRow,
    height: adw::SpinRow,
    fsr: adw::SwitchRow,
    fps: adw::SpinRow,
}

impl GamescopeFields {
    /// Build from `g`.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, game: &Game, title: Option<&str>) -> Self {
        use bigame_core::gamescope::{Config, Filter, FrameLimit, Mode};
        let group = adw::PreferencesGroup::new();
        if let Some(t) = title {
            group.set_title(t);
        }
        if !m.gamescope {
            group.add(&missing_row(
                "Gamescope",
                &i18n("Not installed. It wraps the game in a micro-compositor: scaling, a frame limit, a stable fullscreen."),
                "sudo pacman -S gamescope",
            ));
            return Self { group, rows: None };
        }
        let general_on = m.video.upscaling.gamescope_enabled;
        let subtitle = if game.steam {
            i18n("Automatic: only when this game's own settings below need it")
        } else if general_on {
            i18n("Automatic follows Tuning, where Gamescope is on")
        } else {
            i18n("Automatic follows Tuning, where Gamescope is off")
        };
        let items = [
            ("auto".to_owned(), i18n("Automatic")),
            ("enabled".to_owned(), i18n("Always")),
            ("disabled".to_owned(), i18n("Never")),
        ];
        let mode = Picker::new(
            &i18n("Use Gamescope"),
            &subtitle,
            &items,
            match g.profile.gamescope_mode {
                Mode::Auto => "auto",
                Mode::Enabled => "enabled",
                Mode::Disabled => "disabled",
            },
        )
        .row;
        group.add(&mode);
        if game.steam {
            let steam = Notice::new(
                Kind::Info,
                &i18n("The Steam client starts this game"),
                &i18n(
                    "Always, or this game's own settings, go into its Steam launch options when you save, with Steam closed; your own options there are kept.",
                ),
            );
            group.add(steam.widget());
        }

        // Pre-filled with the general sizes when the game has none of its own.
        let cfg = g
            .profile
            .gamescope
            .clone()
            .unwrap_or_else(bigame_core::gamescope::load_global);
        let own = adw::ExpanderRow::builder()
            .title(i18n("This game's own Gamescope settings"))
            .subtitle(i18n("Off: the sizes and filter from Tuning"))
            .show_enable_switch(true)
            .enable_expansion(g.profile.gamescope.is_some())
            .build();
        let spin = |title: &str, value: u32, min: f64, max: f64| {
            let row = adw::SpinRow::new(
                Some(&gtk4::Adjustment::new(
                    f64::from(value).clamp(min, max),
                    min,
                    max,
                    1.0,
                    10.0,
                    0.0,
                )),
                1.0,
                0,
            );
            row.set_title(title);
            row
        };
        let width = spin(&i18n("Render width"), cfg.render_width, 640.0, 7680.0);
        let height = spin(&i18n("Render height"), cfg.render_height, 480.0, 4320.0);
        let fsr = adw::SwitchRow::builder()
            .title(i18n("FSR upscaling filter"))
            .active(cfg.filter == Filter::Fsr)
            .build();
        let fps = spin(
            &i18n("Frame limit (0 = none)"),
            match cfg.frame_limit {
                FrameLimit::NestedRefresh(hz) => hz,
                FrameLimit::None => 0,
            },
            0.0,
            500.0,
        );
        for r in [&width, &height] {
            own.add_row(r);
        }
        own.add_row(&fsr);
        own.add_row(&fps);
        group.add(&own);

        // What Automatic decides, from `gamescope --help` (probed once, off
        // the main thread), and the pairs that collide at launch.
        let explain = adw::ActionRow::builder()
            .title(i18n("What Automatic does here"))
            .subtitle(i18n("Checking…"))
            .use_markup(false)
            .visible(false)
            .build();
        explain.set_subtitle_lines(0);
        explain.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
        group.add(&explain);
        let collide = Notice::new(Kind::Info, "", "");
        collide.set_visible(false);
        group.add(collide.widget());

        let caps: Rc<RefCell<Option<Option<bigame_core::capabilities::GamescopeCaps>>>> =
            Rc::new(RefCell::new(None));
        let wine_on = m.video.upscaling.wine_fsr_enabled;
        let optiscaler = game.optiscaler.contains(&Feature::OptiScalerUpscaling);
        let refresh = {
            let (explain, collide, caps) = (explain.clone(), collide.clone(), Rc::clone(&caps));
            let (mode, own, width, height, fsr, fps) = (
                mode.clone(),
                own.clone(),
                width.clone(),
                height.clone(),
                fsr.clone(),
                fps.clone(),
            );
            Rc::new(move || {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let cfg = Config {
                    render_width: width.value() as u32,
                    render_height: height.value() as u32,
                    filter: if fsr.is_active() {
                        Filter::Fsr
                    } else {
                        Filter::Linear
                    },
                    frame_limit: match fps.value() as u32 {
                        0 => FrameLimit::None,
                        hz => FrameLimit::NestedRefresh(hz),
                    },
                    ..Config::default()
                };
                let scales = own.enables_expansion() && mode.selected() != 2;
                if let Some(caps) = caps.borrow().as_ref() {
                    let d = bigame_core::gamescope::decide(
                        Mode::Auto,
                        &cfg,
                        caps.as_ref(),
                        bigame_core::hardware::detect_session(),
                    );
                    explain.set_subtitle(&format!(
                        "{} — {}",
                        if d.use_gamescope {
                            i18n("Gamescope would run")
                        } else {
                            i18n("Gamescope would not run")
                        },
                        tr(&d.reason)
                    ));
                    explain.set_visible(mode.selected() == 0);
                }
                if scales && optiscaler {
                    collide.set(
                        Kind::Info,
                        &i18n("OptiScaler already upscales this game"),
                        &i18n("At launch Gamescope runs it at the game's own size, so the image is not scaled twice."),
                    );
                    collide.set_visible(true);
                } else if scales && wine_on {
                    collide.set(
                        Kind::Info,
                        &i18n("Wine FSR is on in Tuning"),
                        &i18n("While Gamescope upscales this game, Wine FSR is switched off for it at launch, so the image is not scaled twice."),
                    );
                    collide.set_visible(true);
                } else {
                    collide.set_visible(false);
                }
            })
        };
        {
            let refresh = Rc::clone(&refresh);
            glib::spawn_future_local(async move {
                let probed = gio::spawn_blocking(bigame_core::capabilities::gamescope_cached)
                    .await
                    .ok()
                    .flatten();
                *caps.borrow_mut() = Some(probed);
                refresh();
            });
        }
        refresh();
        for w in [&width, &height, &fps] {
            let refresh = Rc::clone(&refresh);
            w.connect_value_notify(move |_| refresh());
        }
        {
            let refresh = Rc::clone(&refresh);
            fsr.connect_active_notify(move |_| refresh());
        }
        {
            let refresh = Rc::clone(&refresh);
            mode.connect_selected_notify(move |_| refresh());
        }
        {
            let refresh = Rc::clone(&refresh);
            own.connect_enable_expansion_notify(move |_| refresh());
        }
        Self {
            group,
            rows: Some(GamescopeRows {
                mode,
                own,
                width,
                height,
                fsr,
                fps,
            }),
        }
    }

    /// Write into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        use bigame_core::gamescope::{Config, Filter, FrameLimit, Mode};
        let Some(r) = &self.rows else {
            return;
        };
        g.profile.gamescope_mode = match r.mode.selected() {
            1 => Mode::Enabled,
            2 => Mode::Disabled,
            _ => Mode::Auto,
        };
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        {
            g.profile.gamescope = r.own.enables_expansion().then(|| Config {
                render_width: r.width.value() as u32,
                render_height: r.height.value() as u32,
                filter: if r.fsr.is_active() {
                    Filter::Fsr
                } else {
                    Filter::Linear
                },
                frame_limit: match r.fps.value() as u32 {
                    0 => FrameLimit::None,
                    hz => FrameLimit::NestedRefresh(hz),
                },
                ..g.profile.gamescope.clone().unwrap_or_default()
            });
        }
    }

    /// Whether Gamescope is installed.
    #[must_use]
    pub fn available(&self) -> bool {
        self.rows.is_some()
    }
}

/// lsfg-vk for one game, read from and written to lsfg-vk's own file.
pub struct FrameGenFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    rows: Option<FrameGenRows>,
}

struct FrameGenRows {
    on: adw::SwitchRow,
    multiplier: adw::SpinRow,
    flow: adw::SpinRow,
    perf: adw::SwitchRow,
    hdr: adw::SwitchRow,
    present: adw::ComboRow,
}

impl FrameGenFields {
    /// Build from `g`. `on_use_lsfg` runs when the user picks lsfg-vk over
    /// `OptiScaler`'s frame generation in this game.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn build(
        g: &GameOptimization,
        m: &Machine,
        game: &Game,
        title: Option<&str>,
        on_use_lsfg: impl Fn(&gtk4::Widget) + 'static,
    ) -> Self {
        let group = adw::PreferencesGroup::new();
        if let Some(t) = title {
            group.set_title(t);
        }
        if !m.lsfg {
            group.add(&missing_row(
                "lsfg-vk",
                &i18n("Not installed. Generates extra frames between rendered ones (Lossless Scaling's method, as a Vulkan layer). It needs your own Lossless.dll."),
                "sudo pacman -S lsfg-vk",
            ));
            return Self { group, rows: None };
        }
        let f = g.frame_generation;
        let on = adw::SwitchRow::builder()
            .title(i18n("Generate frames with lsfg-vk"))
            .subtitle(i18n(
                "Raises the presented frame rate, not the rendered one, and adds latency. On or off takes effect at the game's next start.",
            ))
            .active(f.on())
            .sensitive(m.lsfg_dll || f.on())
            .build();
        group.add(&on);
        let spin = |title: &str, value: u32, min: f64, max: f64| {
            let row = adw::SpinRow::new(
                Some(&gtk4::Adjustment::new(
                    f64::from(value).clamp(min, max),
                    min,
                    max,
                    1.0,
                    5.0,
                    0.0,
                )),
                1.0,
                0,
            );
            row.set_title(title);
            row
        };
        let multiplier = spin(&i18n("Multiplier"), f.multiplier.max(2), 2.0, 20.0);
        multiplier.set_subtitle(&i18n("Frames shown per rendered frame"));
        let flow = spin(&i18n("Flow scale (%)"), f.flow_scale, 25.0, 100.0);
        flow.set_subtitle(&i18n("Motion estimation resolution. Lower = faster."));
        let perf = adw::SwitchRow::builder()
            .title(i18n("Performance mode"))
            .subtitle(i18n("lsfg-vk's lighter model"))
            .active(f.performance)
            .build();
        let hdr = adw::SwitchRow::builder()
            .title(i18n("HDR Mode"))
            .active(f.hdr)
            .build();
        let presents = gtk4::StringList::new(&[
            &i18n("VSync/FIFO (default)"),
            &i18n("Recommended"),
            &i18n("Mailbox"),
            &i18n("Immediate"),
        ]);
        let present = adw::ComboRow::builder()
            .title(i18n("Present Mode"))
            .model(&presents)
            .selected(f.present_mode.min(3))
            .build();
        let details = adw::ExpanderRow::builder()
            .title(i18n("lsfg-vk options"))
            .subtitle(i18n("Multiplier, flow scale, HDR and present mode"))
            .build();
        for r in [&multiplier, &flow] {
            details.add_row(r);
        }
        details.add_row(&perf);
        details.add_row(&hdr);
        details.add_row(&present);
        group.add(&details);

        // What stands between this choice and the game.
        let note = Notice::new(Kind::Info, "", "");
        group.add(note.widget());
        let general_on = opt::lsfg_general_on(&m.video);
        let dll = m.lsfg_dll;
        let optiscaler_fg = game.optiscaler.contains(&Feature::OptiScalerFrameGen);
        let asking = Rc::new(Cell::new(false));
        let on_use_lsfg: Rc<dyn Fn(&gtk4::Widget)> = Rc::new(on_use_lsfg);
        // Both on — as an older version could leave a game — is named with
        // the two ways out; a game its launcher starts would run both.
        let explain: Rc<dyn Fn(bool)> = {
            let (note, details, on) = (note.clone(), details.clone(), on.clone());
            let (asking, use_lsfg) = (Rc::clone(&asking), Rc::clone(&on_use_lsfg));
            Rc::new(move |active: bool| {
                details.set_sensitive(active);
                note.clear_actions();
                if !dll {
                    note.set(
                        Kind::Warning,
                        &i18n("lsfg-vk needs your Lossless.dll"),
                        &i18n("Set its path in Tuning → Frame generation, then come back here."),
                    );
                    note.set_visible(true);
                } else if active && optiscaler_fg {
                    note.set(
                        Kind::Conflict,
                        &i18n("OptiScaler and lsfg-vk both generate frames in this game"),
                        &i18n("Two frame generators at once cause artefacts, added latency and unpredictable behaviour. Keep one."),
                    );
                    {
                        let (on, asking) = (on.clone(), Rc::clone(&asking));
                        note.add_action(
                            &i18n("Keep %s")
                                .replace("%s", &notice::feature_name(Feature::OptiScalerFrameGen)),
                            false,
                            move |_| {
                                asking.set(true);
                                on.set_active(false);
                                asking.set(false);
                            },
                        );
                    }
                    {
                        let use_lsfg = Rc::clone(&use_lsfg);
                        note.add_action(&i18n("Use %s").replace("%s", "lsfg-vk"), true, move |b| {
                            use_lsfg(b.upcast_ref());
                        });
                    }
                    note.set_visible(true);
                } else if active && !general_on {
                    note.set(
                        Kind::Info,
                        &i18n("lsfg-vk is off in Tuning"),
                        &i18n("This game's setting is kept, and applies when lsfg-vk is turned on in Tuning."),
                    );
                    note.set_visible(true);
                } else {
                    note.set_visible(false);
                }
            })
        };
        explain(on.is_active());
        // Two frame generators never run together: switching lsfg-vk on
        // where OptiScaler generates frames asks which one to keep.
        {
            let explain = Rc::clone(&explain);
            on.connect_active_notify(move |row| {
                explain(row.is_active());
                if asking.get() || !row.is_active() || !optiscaler_fg {
                    return;
                }
                let Some(c) = opt::conflict(Feature::LsfgVk, Feature::OptiScalerFrameGen) else {
                    return;
                };
                let (row2, asking2, use_lsfg) =
                    (row.clone(), Rc::clone(&asking), Rc::clone(&on_use_lsfg));
                notice::ask_conflict(row, &c, move |use_requested| {
                    if use_requested {
                        use_lsfg(row2.upcast_ref());
                    } else {
                        asking2.set(true);
                        row2.set_active(false);
                        asking2.set(false);
                    }
                });
            });
        }
        Self {
            group,
            rows: Some(FrameGenRows {
                on,
                multiplier,
                flow,
                perf,
                hdr,
                present,
            }),
        }
    }

    /// Write into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        let Some(r) = &self.rows else {
            return;
        };
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        {
            g.frame_generation = opt::FrameGeneration {
                multiplier: if r.on.is_active() {
                    r.multiplier.value() as u32
                } else {
                    1
                },
                flow_scale: r.flow.value() as u32,
                performance: r.perf.is_active(),
                hdr: r.hdr.is_active(),
                present_mode: r.present.selected(),
            };
        }
    }

    /// Whether lsfg-vk is installed.
    #[must_use]
    pub fn available(&self) -> bool {
        self.rows.is_some()
    }
}

/// `MangoHud` for one game.
pub struct MangoHudFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    picker: Option<Picker>,
}

impl MangoHudFields {
    /// Build from `g`.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, title: Option<&str>) -> Self {
        use bigame_core::mangohud::Mode;
        let group = adw::PreferencesGroup::new();
        if let Some(t) = title {
            group.set_title(t);
        }
        if !m.mangohud {
            group.add(&missing_row(
                "MangoHud",
                &i18n("Not installed. The performance overlay; it also captures the frametimes Measure the difference uses."),
                "sudo pacman -S mangohud",
            ));
            return Self {
                group,
                picker: None,
            };
        }
        let picker = Picker::new(
            &i18n("Show MangoHud"),
            &i18n("On: Vulkan and Proton games. Forced: OpenGL games too."),
            &[
                ("off".to_owned(), i18n("Off")),
                ("on".to_owned(), i18n("On")),
                ("forced".to_owned(), i18n("Forced")),
            ],
            match g.mangohud {
                Mode::Off => "off",
                Mode::On => "on",
                Mode::Forced => "forced",
            },
        );
        picker.row.add_suffix(&crate::widgets::info::button(
            "MangoHud",
            &i18n("The performance overlay for this game. On uses MangoHud's Vulkan layer, which covers Vulkan and every Proton game; Forced uses its wrapper, which also reaches OpenGL games. It is written where the game's launcher reads it: Steam's launch options (with Steam closed), the game's settings in Heroic (with Heroic closed) or in Lutris."),
        ));
        group.add(&picker.row);
        Self {
            group,
            picker: Some(picker),
        }
    }

    /// Write into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        use bigame_core::mangohud::Mode;
        if let Some(p) = &self.picker {
            g.mangohud = match p.value().as_str() {
                "on" => Mode::On,
                "forced" => Mode::Forced,
                _ => Mode::Off,
            };
        }
    }

    /// Whether `MangoHud` is installed.
    #[must_use]
    pub fn available(&self) -> bool {
        self.picker.is_some()
    }
}

/// Every per-game block, built together.
pub struct GameFields {
    /// Performance mode, idle inhibit.
    pub performance: PerformanceFields,
    /// Scheduler.
    pub scheduler: SchedulerFields,
    /// 3D V-Cache.
    pub vcache: VCacheFields,
    /// Gamescope.
    pub gamescope: GamescopeFields,
    /// lsfg-vk.
    pub frame_generation: FrameGenFields,
    /// `MangoHud`.
    pub mangohud: MangoHudFields,
}

impl GameFields {
    /// Build every block from `g`. `titled` gives each block its section
    /// title (the editor); the wizard titles its steps instead.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, game: &Game, titled: bool) -> Self {
        let t = |s: &str| titled.then(|| i18n(s));
        let target = game.target.clone();
        Self {
            performance: PerformanceFields::build(g, m, t("Performance").as_deref()),
            scheduler: SchedulerFields::build(g, m, None),
            vcache: VCacheFields::build(g, m, None),
            gamescope: GamescopeFields::build(g, m, game, t("Display").as_deref()),
            frame_generation: FrameGenFields::build(
                g,
                m,
                game,
                t("Frame generation").as_deref(),
                move |anchor| {
                    if let Some(target) = target.clone() {
                        crate::views::ai_graphics::open_to_change(anchor, target, |cfg| {
                            cfg.frame_generation =
                                bigame_core::graphics::config::FrameGeneration::Off;
                        });
                    }
                },
            ),
            mangohud: MangoHudFields::build(g, m, t("Monitoring").as_deref()),
        }
    }

    /// Write every block into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        self.performance.apply(g);
        self.scheduler.apply(g);
        self.vcache.apply(g);
        self.gamescope.apply(g);
        self.frame_generation.apply(g);
        self.mangohud.apply(g);
    }
}

/// What a profile holds, as label/value lines: the wizard's review and
/// anything else that summarises a profile. Unavailable features say so
/// rather than disappearing.
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
                Mode::Auto => i18n("Automatic"),
                Mode::Enabled => i18n("Always"),
                Mode::Disabled => i18n("Never"),
            };
            match &g.profile.gamescope {
                Some(c) if g.profile.gamescope_mode != Mode::Disabled => {
                    format!("{mode} · {}×{}", c.render_width, c.render_height)
                }
                _ => mode,
            }
        } else {
            i18n("Not installed")
        },
    ));
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
    let mut note = None;
    if let Some(g) = &report.gamescope {
        use bigame_core::steam_gamescope::Applied as G;
        match g {
            Ok(G::SteamRunning) => problems.push(i18n(
                "Gamescope: close Steam first: it keeps its launch options in memory and would overwrite the change.",
            )),
            Ok(G::Written(opts)) if !opts.is_empty() => {
                note = Some(i18n("Steam launch options: %s").replace("%s", opts));
            }
            Ok(_) => {}
            Err(e) => problems.push(format!("Gamescope: {}", crate::i18n::error_text(e))),
        }
    }
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
}
