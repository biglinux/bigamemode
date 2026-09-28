//! One game's rows: the profile editor's sections, and the wizard's steps.
//!
//! The same sections as Tuning, in the same order and with the same rows
//! and ⓘ (`widgets::launch`, `widgets::resolution`): Performance, Display,
//! Image quality, Frame generation, Monitoring. Every value a game can leave
//! to Tuning has "General configuration" first, and the row says what that
//! is now ("Follows Tuning: …"); what only a game has (keeping the screen
//! awake, a frame limit, lsfg-vk's entry) says so.
//!
//! A row is offered only where it reaches the game ([`Reach`]): falcond's
//! and lsfg-vk's settings reach it however it is started, `MangoHud` goes
//! where its launcher reads it, and Gamescope, Wine FSR and vkBasalt reach
//! it when BiGame-mode starts it, through its Steam launch options, or
//! through its settings in Heroic.
//!
//! Display and Image quality edit one set of values ([`Live`]), so turning
//! on one upscaler where the other already works for this game asks which to
//! keep (`notice::ask_conflict`), as Tuning does.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::game_launch::GameLaunch;
use bigame_core::gamescope::Mode;
use bigame_core::models::{GamescopeFilter, UpscalingSettings};
use bigame_core::optimization::{self as opt, Feature, GameOptimization};

use crate::i18n::{i18n, tr};
use crate::widgets::launch::{self, INHERIT};
use crate::widgets::notice::{self, Kind, Notice};
use crate::widgets::optimization::{
    Game, Machine, Picker, Reach, Scope, missing_row, mode_items, scheduler_items,
    scheduler_summary, scheduler_unavailable_row, vcache_items, vcache_name,
    vcache_unsupported_row,
};
use crate::widgets::resolution::{self, SizePicker};

/// `group`, or a new one: the editor puts several blocks in one section,
/// the wizard shows each on its own.
fn group_in(into: Option<&adw::PreferencesGroup>) -> adw::PreferencesGroup {
    into.cloned().unwrap_or_default()
}

/// What a row does and, while it follows Tuning, what Tuning has now.
fn follow_subtitle(what: &str, general: Option<&str>) -> String {
    match general {
        None => what.to_owned(),
        Some(g) => {
            let follows = i18n("Follows Tuning: %s").replace("%s", g);
            if what.is_empty() {
                follows
            } else {
                format!("{what}\n{follows}")
            }
        }
    }
}

/// On or Off, as a word.
fn on_off(on: bool) -> String {
    if on { i18n("On") } else { i18n("Off") }
}

/// The three choices of a setting a game can leave to Tuning, turn on or
/// turn off.
fn tri_items() -> Vec<(String, String)> {
    vec![
        (INHERIT.to_owned(), i18n("General configuration")),
        ("on".to_owned(), i18n("On")),
        ("off".to_owned(), i18n("Off")),
    ]
}

fn tri_id(v: Option<bool>) -> &'static str {
    match v {
        None => INHERIT,
        Some(true) => "on",
        Some(false) => "off",
    }
}

fn tri_of(id: &str) -> Option<bool> {
    match id {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// The row that says a game's launch settings cannot reach it.
fn no_reach_row() -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(i18n("Not for this game"))
        .subtitle(i18n(
            "BiGame-mode cannot start this game, and its launcher takes no launch settings from BiGame-mode: it gets Tuning's, Wine FSR and vkBasalt from the session environment.",
        ))
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    row.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
    row
}

// ── The values Display and Image quality share ──────────────────────────────

/// The values before a change, to put back.
type Before = (GameLaunch, Mode);

/// Checks a change the user made, given the values before it.
type Check = Rc<dyn Fn(Before, &gtk4::Widget)>;

/// A game's launch values as its rows edit them. Display and Image quality
/// change the same ones, so a change in one is checked against the other
/// before it stands.
#[derive(Default)]
struct Live {
    launch: RefCell<GameLaunch>,
    mode: Cell<Mode>,
    /// Set while the page itself moves the rows.
    quiet: Cell<bool>,
    /// Set while the user is asked which of two upscalers to keep: the
    /// values hold both until then, and that is a question, not a state.
    asking: Cell<bool>,
    /// Checks a change the user made, given the values before it.
    check: RefCell<Option<Check>>,
    /// Put each row back to the values above.
    resync: RefCell<Vec<Box<dyn Fn()>>>,
    /// Bring subtitles, sensitivity and notices up to date.
    refresh: RefCell<Vec<Box<dyn Fn()>>>,
}

impl Live {
    fn snapshot(&self) -> Before {
        (*self.launch.borrow(), self.mode.get())
    }

    /// A change the user made on `anchor`'s row.
    fn edit(&self, anchor: &gtk4::Widget, f: impl FnOnce(&mut GameLaunch, &Cell<Mode>)) {
        if self.quiet.get() {
            return;
        }
        let before = self.snapshot();
        f(&mut self.launch.borrow_mut(), &self.mode);
        // The check first: it says whether a question is open, which the
        // notices read.
        let check = self.check.borrow().clone();
        if let Some(check) = check {
            check(before, anchor);
        }
        self.refresh();
    }

    /// A change the page makes: the rows follow it.
    fn set(&self, f: impl FnOnce(&mut GameLaunch, &Cell<Mode>)) {
        f(&mut self.launch.borrow_mut(), &self.mode);
        self.quiet.set(true);
        for resync in self.resync.borrow().iter() {
            resync();
        }
        self.quiet.set(false);
        self.refresh();
    }

    fn restore(&self, before: Before) {
        self.set(|l, m| {
            *l = before.0;
            m.set(before.1);
        });
    }

    fn refresh(&self) {
        for refresh in self.refresh.borrow().iter() {
            refresh();
        }
    }

    fn on_resync(&self, f: impl Fn() + 'static) {
        self.resync.borrow_mut().push(Box::new(f));
    }

    fn on_refresh(&self, f: impl Fn() + 'static) {
        f();
        self.refresh.borrow_mut().push(Box::new(f));
    }
}

/// Whether Gamescope upscales the game and whether Wine FSR is on for it,
/// with `launch` and `mode`. `OptiScaler` upscaling it takes both out: its
/// launch switches them off.
fn upscalers(general: &UpscalingSettings, game: &Game, before: Before) -> (bool, bool) {
    if game.optiscaler.contains(&Feature::OptiScalerUpscaling) {
        return (false, false);
    }
    let (launch, mode) = before;
    (
        launch.upscales(general, mode, !game.reach.launcher_written()),
        launch.over(general, mode).wine_fsr_enabled,
    )
}

// ── Performance ─────────────────────────────────────────────────────────────

/// Performance mode and idle inhibit.
pub struct PerformanceFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    perf: adw::SwitchRow,
    idle: adw::SwitchRow,
}

impl PerformanceFields {
    /// Build from `g`, into `into` or a group of its own.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, into: Option<&adw::PreferencesGroup>) -> Self {
        let group = group_in(into);
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
    /// Build from `g`, into `into` or a group of its own.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, into: Option<&adw::PreferencesGroup>) -> Self {
        let group = group_in(into);
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
    /// Build from `g`, into `into` or a group of its own.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, into: Option<&adw::PreferencesGroup>) -> Self {
        let group = group_in(into);
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
                row.set_subtitle(&follow_subtitle(
                    &i18n("Which CCD this game prefers"),
                    opt::inherits(v).then_some(general.as_str()),
                ));
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

// ── Display (Gamescope) ─────────────────────────────────────────────────────

/// The frame limits offered, in frames per second.
const FRAME_LIMITS: &[u32] = &[30, 40, 45, 50, 60, 72, 75, 90, 100, 120, 144, 165, 240];

/// Gamescope for one game: whether it runs, and each of Tuning's values the
/// game may set for itself, in Tuning's order.
pub struct GamescopeFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    available: bool,
}

impl GamescopeFields {
    #[allow(clippy::too_many_lines)]
    fn build(
        m: &Machine,
        game: &Game,
        live: &Rc<Live>,
        into: Option<&adw::PreferencesGroup>,
    ) -> Self {
        let group = group_in(into);
        // Heroic's Flatpak runs its own Gamescope, from Flathub's extension:
        // without it, Heroic starts the game without Gamescope, so nothing
        // of Gamescope's is offered, rather than a setting that does nothing.
        let heroic_flatpak = game.reach.heroic_flatpak();
        if let Some(command) = heroic_flatpak
            .then(bigame_core::heroic_launch::flatpak_gamescope_missing)
            .flatten()
        {
            group.add(&missing_row(
                "Gamescope",
                &i18n(
                    "Heroic's Flatpak finds Gamescope only in Flathub's Gamescope extension for its runtime, which is not installed. Heroic would start this game without it, so nothing of Gamescope's is written for it. Install it and restart Heroic:",
                ),
                &command,
            ));
            return Self {
                group,
                available: false,
            };
        }
        if !m.gamescope && !heroic_flatpak {
            group.add(&missing_row(
                "Gamescope",
                &i18n("Not installed. It wraps the game in a micro-compositor: scaling, a frame limit, a stable fullscreen."),
                "sudo pacman -S gamescope",
            ));
            return Self {
                group,
                available: false,
            };
        }
        if game.reach == Reach::Nothing {
            group.add(&no_reach_row());
            return Self {
                group,
                available: false,
            };
        }
        let general = m.video.upscaling.clone();
        let steam = game.reach == Reach::Steam;
        let heroic = matches!(game.reach, Reach::Heroic { .. });
        // Its launcher's settings carry it: Automatic runs Gamescope only
        // for the game's own values.
        let written = game.reach.launcher_written();
        let own = *live.launch.borrow();

        // Whether it runs: the general configuration, Always or Never.
        let mode_id = |m: Mode| match m {
            Mode::Auto => INHERIT,
            Mode::Enabled => "enabled",
            Mode::Disabled => "disabled",
        };
        let mode = Picker::new(
            "Gamescope",
            "",
            &[
                (INHERIT.to_owned(), i18n("General configuration")),
                ("enabled".to_owned(), i18n("Always")),
                ("disabled".to_owned(), i18n("Never")),
            ],
            mode_id(live.mode.get()),
        );
        mode.row
            .add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
        mode.row.set_subtitle_lines(4);
        group.add(&mode.row);

        let filter = launch::filter_picker(true, own.filter);
        filter.row.set_subtitle_lines(4);
        group.add(&filter.row);

        let mut sharpness_items = vec![(INHERIT.to_owned(), i18n("General configuration"))];
        sharpness_items.extend((0..=20u8).map(|n| (n.to_string(), n.to_string())));
        let sharpness = Picker::new(
            &i18n("FSR sharpness"),
            &i18n("0 = sharpest · 20 = softest"),
            &sharpness_items,
            &own.sharpness
                .map_or_else(|| INHERIT.to_owned(), |s| s.to_string()),
        );
        sharpness.row.set_subtitle_lines(4);
        group.add(&sharpness.row);

        let general_label = i18n("General configuration");
        let render = SizePicker::inheriting(
            &i18n("Render size"),
            &i18n("The game draws at this size"),
            &general_label,
            &i18n("The game's own size"),
            own.render,
        );
        render.with_main_screen(false);
        render.row.set_subtitle_lines(4);
        group.add(&render.row);
        let output = SizePicker::inheriting(
            &i18n("Output size"),
            &i18n("Upscaled to this size, usually the screen's"),
            &general_label,
            &i18n("The same as the render size"),
            own.output,
        );
        output.with_main_screen(true);
        output.row.set_subtitle_lines(4);
        group.add(&output.row);

        // Only a game has a frame limit: Tuning has none for every game.
        let mut limits = vec![("0".to_owned(), i18n("None"))];
        let mut rates: Vec<u32> = FRAME_LIMITS.to_vec();
        if own.frame_limit > 0 && !rates.contains(&own.frame_limit) {
            rates.push(own.frame_limit);
            rates.sort_unstable();
        }
        limits.extend(rates.iter().map(|r| (r.to_string(), format!("{r} FPS"))));
        let frame = Picker::new(
            &i18n("Frame limit"),
            &i18n(
                "Only this game: Gamescope shows it at this rate (-r). A Turbo preset with a frame limit replaces it.",
            ),
            &limits,
            &own.frame_limit.to_string(),
        );
        frame.row.set_subtitle_lines(4);
        frame
            .row
            .add_prefix(&gtk4::Image::from_icon_name("speedometer-symbolic"));
        group.add(&frame.row);

        // What happens at launch, as the launch decides it (from
        // `gamescope --help`, probed once off the main thread).
        let explain = adw::ActionRow::builder()
            .title(i18n("At launch"))
            .subtitle(i18n("Checking…"))
            .use_markup(false)
            .build();
        explain.set_subtitle_lines(0);
        explain.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
        group.add(&explain);
        let collide = Notice::new(
            Kind::Info,
            &i18n("OptiScaler already upscales this game"),
            &i18n(
                "At launch Gamescope runs it at the game's own size, so the image is not scaled twice.",
            ),
        );
        group.add(collide.widget());
        // Each row writes its value; the page's own moves are quiet.
        {
            let (live, row) = (Rc::clone(live), mode.row.clone());
            mode.connect_changed(move |id| {
                let m = match id {
                    "enabled" => Mode::Enabled,
                    "disabled" => Mode::Disabled,
                    _ => Mode::Auto,
                };
                live.edit(row.upcast_ref(), |_, mode| mode.set(m));
            });
        }
        {
            let (live, row) = (Rc::clone(live), filter.row.clone());
            filter.connect_changed(move |id| {
                let f = launch::filter_of(id);
                live.edit(row.upcast_ref(), |l, _| l.filter = f);
            });
        }
        {
            let (live, row) = (Rc::clone(live), sharpness.row.clone());
            sharpness.connect_changed(move |id| {
                let s = id.parse().ok();
                live.edit(row.upcast_ref(), |l, _| l.sharpness = s);
            });
        }
        {
            let (live, row) = (Rc::clone(live), render.row.clone());
            render.connect_choice(move |c| live.edit(row.upcast_ref(), |l, _| l.render = c));
        }
        {
            let (live, row) = (Rc::clone(live), output.row.clone());
            output.connect_choice(move |c| live.edit(row.upcast_ref(), |l, _| l.output = c));
        }
        {
            let (live, row) = (Rc::clone(live), frame.row.clone());
            frame.connect_changed(move |id| {
                let hz = id.parse().unwrap_or(0);
                live.edit(row.upcast_ref(), |l, _| l.frame_limit = hz);
            });
        }
        {
            let (l, mode, filter, sharpness) = (
                Rc::downgrade(live),
                mode.clone(),
                filter.clone(),
                sharpness.clone(),
            );
            let (render, output, frame) = (Rc::clone(&render), Rc::clone(&output), frame.clone());
            live.on_resync(move || {
                let Some(live) = l.upgrade() else { return };
                let (own, m) = live.snapshot();
                mode.set(mode_id(m));
                filter.set(own.filter.map_or(INHERIT, |f| match f {
                    GamescopeFilter::Fsr => "fsr",
                    GamescopeFilter::Nis => "nis",
                    GamescopeFilter::Integer => "integer",
                }));
                sharpness.set(
                    &own.sharpness
                        .map_or_else(|| INHERIT.to_owned(), |s| s.to_string()),
                );
                render.set_choice(own.render);
                output.set_choice(own.output);
                frame.set(&own.frame_limit.to_string());
            });
        }

        // Subtitles say what the general configuration is now; the
        // explanation and the notice follow every change.
        let caps: Rc<RefCell<Option<Option<bigame_core::capabilities::GamescopeCaps>>>> =
            Rc::new(RefCell::new(None));
        let refresh: Rc<dyn Fn()> = {
            let l = Rc::downgrade(live);
            let (mode_row, filter_row, sharpness_row) =
                (mode.row.clone(), filter.row.clone(), sharpness.row.clone());
            let (render_row, output_row) = (render.row.clone(), output.row.clone());
            let (explain, collide, caps) = (explain.clone(), collide.clone(), Rc::clone(&caps));
            let game = game.clone();
            Rc::new(move || {
                let Some(live) = l.upgrade() else { return };
                let (own, mode) = live.snapshot();
                // Tuning's Gamescope values apply where Gamescope is on
                // there, or where the game says Always.
                let applies = mode == Mode::Enabled || general.gamescope_enabled;
                let off_there = i18n("nothing, Gamescope is off there");
                let or_off = |v: String| if applies { v } else { off_there.clone() };
                mode_row.set_subtitle(&if written {
                    let what = if steam {
                        i18n("Written into this game's Steam launch options")
                    } else {
                        i18n("Written into Heroic's settings for this game, with Heroic closed")
                    };
                    if mode == Mode::Auto {
                        format!(
                            "{what}\n{}",
                            i18n("General configuration: only when this game's own values below need Gamescope")
                        )
                    } else {
                        what
                    }
                } else {
                    follow_subtitle(
                        &i18n("Wraps this game when BiGame-mode starts it"),
                        (mode == Mode::Auto)
                            .then(|| on_off(general.gamescope_enabled))
                            .as_deref(),
                    )
                });
                filter_row.set_subtitle(&follow_subtitle(
                    &i18n("Used when the render size is below the output size"),
                    own.filter
                        .is_none()
                        .then(|| or_off(launch::filter_name(general.gamescope_filter)))
                        .as_deref(),
                ));
                let effective = own.over(&general, mode);
                let fsr = if own.filter.is_some() || applies {
                    effective.gamescope_filter == GamescopeFilter::Fsr
                } else {
                    false
                };
                sharpness_row.set_sensitive(fsr);
                let scale = i18n("0 = sharpest · 20 = softest");
                sharpness_row.set_subtitle(&follow_subtitle(
                    &if heroic {
                        format!(
                            "{scale}\n{}",
                            i18n("Heroic has no sharpness setting: it goes into Heroic's additional Gamescope options for this game")
                        )
                    } else {
                        scale
                    },
                    own.sharpness
                        .is_none()
                        .then(|| general.clamped_sharpness().to_string())
                        .as_deref(),
                ));
                let size_label = |size: (u32, u32), none: String| {
                    if size.0 == 0 || size.1 == 0 {
                        none
                    } else {
                        resolution::label(size, None)
                    }
                };
                render_row.set_subtitle(&follow_subtitle(
                    &i18n("The game draws at this size"),
                    own.render
                        .is_none()
                        .then(|| {
                            or_off(size_label(
                                (general.base_width, general.base_height),
                                i18n("The game's own size"),
                            ))
                        })
                        .as_deref(),
                ));
                output_row.set_subtitle(&follow_subtitle(
                    &i18n("Upscaled to this size, usually the screen's"),
                    own.output
                        .is_none()
                        .then(|| {
                            or_off(size_label(
                                (general.target_width, general.target_height),
                                i18n("The same as the render size"),
                            ))
                        })
                        .as_deref(),
                ));
                if let Some(caps) = caps.borrow().as_ref() {
                    let d = own.decide(
                        &general,
                        mode,
                        written,
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
                }
                let optiscaler = game.optiscaler.contains(&Feature::OptiScalerUpscaling);
                collide.set_visible(optiscaler && own.upscales(&general, mode, !written));
            })
        };
        {
            let refresh = Rc::clone(&refresh);
            live.on_refresh(move || refresh());
        }
        glib::spawn_future_local(async move {
            // Heroic's Flatpak runs the extension's Gamescope, found above.
            let probed = if heroic_flatpak {
                Some(bigame_core::capabilities::GamescopeCaps::default())
            } else {
                gio::spawn_blocking(bigame_core::capabilities::gamescope_cached)
                    .await
                    .ok()
                    .flatten()
            };
            *caps.borrow_mut() = Some(probed);
            refresh();
        });
        Self {
            group,
            available: true,
        }
    }

    /// Whether this game can have Gamescope settings of its own.
    #[must_use]
    pub fn available(&self) -> bool {
        self.available
    }
}

// ── Image quality ───────────────────────────────────────────────────────────

/// Wine FSR, vkBasalt and AI Graphics for one game.
pub struct ImageQualityFields {
    /// The group.
    pub group: adw::PreferencesGroup,
}

impl ImageQualityFields {
    #[allow(clippy::too_many_lines)]
    fn build(
        m: &Machine,
        game: &Game,
        live: &Rc<Live>,
        into: Option<&adw::PreferencesGroup>,
    ) -> Self {
        let group = group_in(into);
        let general = m.video.upscaling.clone();
        let own = *live.launch.borrow();
        let optiscaler = game.optiscaler.contains(&Feature::OptiScalerUpscaling);
        let heroic = matches!(game.reach, Reach::Heroic { .. });
        let heroic_flatpak = game.reach.heroic_flatpak();

        if game.reach == Reach::Nothing {
            group.add(&no_reach_row());
        } else {
            // Both on for this game — as a profile from before the question
            // could leave it — is named with the two ways out.
            let legacy = Notice::new(
                Kind::Conflict,
                &i18n("Wine FSR and Gamescope upscaling are both on"),
                &i18n(
                    "Two upscalers in series scale the image twice. Until one is chosen, Wine FSR is switched off at launch for games Gamescope upscales.",
                ),
            );
            group.add(legacy.widget());
            {
                let l = Rc::downgrade(live);
                legacy.add_action(
                    &i18n("Keep %s")
                        .replace("%s", &notice::feature_name(Feature::GamescopeUpscaling)),
                    false,
                    move |_| {
                        if let Some(live) = l.upgrade() {
                            live.set(|l, _| l.wine_fsr = Some(false));
                        }
                    },
                );
            }
            {
                let l = Rc::downgrade(live);
                legacy.add_action(&i18n("Use %s").replace("%s", "Wine FSR"), true, move |_| {
                    if let Some(live) = l.upgrade() {
                        live.set(|l, _| l.render = Some((0, 0)));
                    }
                });
            }

            let wine = Picker::new("Wine FSR", "", &tri_items(), tri_id(own.wine_fsr));
            wine.row
                .add_prefix(&gtk4::Image::from_icon_name("zoom-in-symbolic"));
            wine.row.set_subtitle_lines(4);
            group.add(&wine.row);
            let quality = launch::wine_fsr_mode_picker(true, own.wine_fsr_mode);
            quality.row.set_subtitle_lines(4);
            group.add(&quality.row);
            if optiscaler {
                // Its launch switches Wine FSR off; a choice here would be
                // a control that does nothing.
                for row in [&wine.row, &quality.row] {
                    row.set_sensitive(false);
                }
            }
            {
                let (live, row) = (Rc::clone(live), wine.row.clone());
                wine.connect_changed(move |id| {
                    let v = tri_of(id);
                    live.edit(row.upcast_ref(), |l, _| l.wine_fsr = v);
                });
            }
            {
                let (live, row) = (Rc::clone(live), quality.row.clone());
                quality.connect_changed(move |id| {
                    let v = launch::wine_fsr_mode_of(id);
                    live.edit(row.upcast_ref(), |l, _| l.wine_fsr_mode = v);
                });
            }

            // Heroic's Flatpak loads Flathub's vkBasalt, not the system's,
            // and cannot see the user's configuration folder.
            let flatpak_note = heroic_flatpak.then(|| {
                match bigame_core::heroic_launch::flatpak_vkbasalt_missing() {
                    Some(command) => i18n(
                        "Heroic's Flatpak needs Flathub's vkBasalt extension for this to take effect, and it is not installed. Install it and restart Heroic: %s",
                    )
                    .replace("%s", &command),
                    None => i18n(
                        "In Heroic's Flatpak, vkBasalt reads its own file inside Heroic's folder, not the look chosen in Tuning: without one it uses its own sharpening (CAS).",
                    ),
                }
            });
            let vkbasalt = if heroic_flatpak || bigame_core::capabilities::vkbasalt_installed() {
                let vkb = Picker::new("vkBasalt", "", &tri_items(), tri_id(own.vkbasalt));
                vkb.row
                    .add_prefix(&gtk4::Image::from_icon_name("image-x-generic-symbolic"));
                vkb.row.set_subtitle_lines(if heroic { 7 } else { 4 });
                group.add(&vkb.row);
                // The look is vkBasalt's own file, the same for every game.
                let look = adw::ActionRow::builder()
                    .title(i18n("Look"))
                    .subtitle(
                        i18n("%s — chosen in Tuning → Image quality, for every game")
                            .replace("%s", &launch::vkbasalt_look_name()),
                    )
                    .subtitle_lines(3)
                    .use_markup(false)
                    .build();
                look.add_suffix(&launch::vkbasalt_looks_button());
                group.add(&look);
                let (live, row) = (Rc::clone(live), vkb.row.clone());
                vkb.connect_changed(move |id| {
                    let v = tri_of(id);
                    live.edit(row.upcast_ref(), |l, _| l.vkbasalt = v);
                });
                Some(vkb)
            } else {
                group.add(&missing_row(
                    "vkBasalt",
                    &i18n(
                        "Not installed. Visual filters (sharpening, colour) for Vulkan and Proton games.",
                    ),
                    "sudo pacman -S vkbasalt",
                ));
                None
            };

            {
                let l = Rc::downgrade(live);
                let (wine, quality, vkbasalt) = (wine.clone(), quality.clone(), vkbasalt.clone());
                live.on_resync(move || {
                    let Some(live) = l.upgrade() else { return };
                    let own = *live.launch.borrow();
                    wine.set(tri_id(own.wine_fsr));
                    quality.set(
                        own.wine_fsr_mode
                            .map_or(INHERIT, bigame_core::game_launch::wine_fsr_mode_word),
                    );
                    if let Some(v) = &vkbasalt {
                        v.set(tri_id(own.vkbasalt));
                    }
                });
            }
            {
                let l = Rc::downgrade(live);
                let game = game.clone();
                // Once resolved, the notice says what is on now instead of
                // vanishing.
                let (conflicted, resolved) = (Cell::new(false), Cell::new(false));
                let (wine_row, quality_row) = (wine.row.clone(), quality.row.clone());
                let vkbasalt_row = vkbasalt.as_ref().map(|v| v.row.clone());
                let what = i18n(
                    "Wine's own upscaling for Proton games in exclusive fullscreen (WINE_FULLSCREEN_FSR=1)",
                );
                live.on_refresh(move || {
                    let Some(live) = l.upgrade() else { return };
                    let (own, mode) = live.snapshot();
                    let effective = own.over(&general, mode);
                    if optiscaler {
                        wine_row.set_subtitle(&i18n(
                            "OptiScaler upscales this game (AI Graphics): Wine FSR is switched off for it at launch.",
                        ));
                    } else if heroic {
                        // Heroic sets WINE_FULLSCREEN_FSR itself, from its
                        // own switch, over the session's.
                        let mut text = i18n(
                            "Wine's own upscaling for Proton games in exclusive fullscreen: Heroic's own Wine FSR switch, written into its settings for this game",
                        );
                        if own.wine_fsr.is_none() {
                            text.push('\n');
                            text.push_str(&i18n(
                                "General configuration: Heroic's own setting for this game, which Heroic applies over the session's",
                            ));
                        }
                        wine_row.set_subtitle(&text);
                        quality_row
                            .set_sensitive(own.wine_fsr.is_none() || effective.wine_fsr_enabled);
                    } else {
                        wine_row.set_subtitle(&follow_subtitle(
                            &what,
                            own.wine_fsr
                                .is_none()
                                .then(|| on_off(general.wine_fsr_enabled))
                                .as_deref(),
                        ));
                        quality_row.set_sensitive(effective.wine_fsr_enabled);
                    }
                    quality_row.set_subtitle(&follow_subtitle(
                        "",
                        own.wine_fsr_mode
                            .is_none()
                            .then(|| launch::wine_fsr_mode_name(general.wine_fsr_mode))
                            .as_deref(),
                    ));
                    if let Some(row) = &vkbasalt_row {
                        let mut what = i18n("Visual filters (sharpening, colour) for Vulkan and Proton games. A look, not a speed-up: it costs a little GPU time. In the game, Home turns it on and off.");
                        if heroic {
                            what.push('\n');
                            what.push_str(&i18n(
                                "Written into Heroic's settings for this game, with Heroic closed",
                            ));
                        }
                        if let Some(note) = &flatpak_note {
                            what.push('\n');
                            what.push_str(note);
                        }
                        row.set_subtitle(&follow_subtitle(
                            &what,
                            own.vkbasalt
                                .is_none()
                                .then(|| on_off(general.vkbasalt_enabled))
                                .as_deref(),
                        ));
                    }
                    let (up, wine_on) = upscalers(&general, &game, (own, mode));
                    if up && wine_on && !live.asking.get() {
                        conflicted.set(true);
                        legacy.set_visible(true);
                    } else if conflicted.get() {
                        conflicted.set(false);
                        resolved.set(true);
                        let kept = if wine_on {
                            "Wine FSR".to_owned()
                        } else {
                            notice::feature_name(Feature::GamescopeUpscaling)
                        };
                        legacy.clear_actions();
                        legacy.set(
                            Kind::Success,
                            &i18n("Resolved: %s is the only upscaler").replace("%s", &kept),
                            "",
                        );
                    } else if !resolved.get() {
                        legacy.set_visible(false);
                    }
                });
            }
        }

        group.add(&ai_graphics_row(game));
        Self { group }
    }
}

/// AI Graphics for one game: it lives on its own page, opened from here.
fn ai_graphics_row(game: &Game) -> adw::ActionRow {
    use bigame_core::overview::State;
    let installed = !game.optiscaler.is_empty();
    let ai = adw::ActionRow::builder()
        .title(i18n("AI Graphics"))
        .subtitle(if game.target.is_none() {
            i18n("Needs the game's install folder, which its launcher does not record")
        } else if installed {
            i18n("OptiScaler installed by BiGame-mode for this game")
        } else {
            i18n("Upscaling and frame generation inside the game, with backup and undo")
        })
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    ai.add_prefix(&gtk4::Image::from_icon_name(
        "applications-science-symbolic",
    ));
    let chip = crate::widgets::status::Chip::new(State::Off);
    if installed {
        chip.set(State::Configured, Some(&i18n("Installed")));
    } else {
        chip.set(State::Off, Some(&i18n("Not set up")));
    }
    ai.add_suffix(chip.widget());
    if let Some(target) = game.target.clone() {
        let open = gtk4::Button::builder()
            .label(i18n("Open"))
            .valign(gtk4::Align::Center)
            .build();
        open.connect_clicked(move |b| {
            crate::views::ai_graphics::open(b, target.clone(), None);
        });
        ai.add_suffix(&open);
    }
    ai
}

// ── Frame generation ────────────────────────────────────────────────────────

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
        into: Option<&adw::PreferencesGroup>,
        on_use_lsfg: impl Fn(&gtk4::Widget) + 'static,
    ) -> Self {
        let group = group_in(into);
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
        let dll = Rc::new(Cell::new(m.lsfg_dll));
        let optiscaler_fg = game.optiscaler.contains(&Feature::OptiScalerFrameGen);
        let asking = Rc::new(Cell::new(false));
        let on_use_lsfg: Rc<dyn Fn(&gtk4::Widget)> = Rc::new(on_use_lsfg);
        // Both on — as an older version could leave a game — is named with
        // the two ways out; a game its launcher starts would run both.
        let explain: Rc<dyn Fn(bool)> = {
            let (note, details, on) = (note.clone(), details.clone(), on.clone());
            let (asking, use_lsfg, dll) =
                (Rc::clone(&asking), Rc::clone(&on_use_lsfg), Rc::clone(&dll));
            Rc::new(move |active: bool| {
                details.set_sensitive(active);
                note.clear_actions();
                if !dll.get() {
                    note.set(
                        Kind::Warning,
                        &i18n("lsfg-vk needs your Lossless.dll"),
                        &i18n("Set its path in the row above: one file for every game, saved at once."),
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
        // The DLL is lsfg-vk's own, shared by every game: the same row as
        // Tuning's, so a missing one is fixed where it is noticed.
        {
            let (explain, on, dll) = (Rc::clone(&explain), on.clone(), Rc::clone(&dll));
            group.add(&crate::widgets::fg_controls::dll_row(move |ready| {
                dll.set(ready);
                on.set_sensitive(ready || on.is_active());
                explain(on.is_active());
            }));
        }
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

// ── Monitoring ──────────────────────────────────────────────────────────────

/// `MangoHud` for one game.
pub struct MangoHudFields {
    /// The group.
    pub group: adw::PreferencesGroup,
    picker: Option<Picker>,
}

impl MangoHudFields {
    /// Build from `g`.
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, into: Option<&adw::PreferencesGroup>) -> Self {
        use bigame_core::mangohud::Mode;
        let group = group_in(into);
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
        picker.row.add_prefix(&gtk4::Image::from_icon_name(
            "utilities-system-monitor-symbolic",
        ));
        picker.row.add_suffix(&crate::widgets::info::button(
            "MangoHud",
            &i18n("The performance overlay for this game. On uses MangoHud's Vulkan layer, which covers Vulkan and every Proton game; Forced uses its wrapper, which also reaches OpenGL games. It is written where the game's launcher reads it: Steam's launch options (with Steam closed), the game's settings in Heroic (with Heroic closed) or in Lutris."),
        ));
        group.add(&picker.row);
        // How it looks is MangoHud's own file, the same for every game.
        let style = adw::ActionRow::builder()
            .title(i18n("Overlay style"))
            .subtitle(
                i18n("%s — chosen in Tuning → Monitoring, for every game")
                    .replace("%s", &launch::mangohud_style_name()),
            )
            .subtitle_lines(3)
            .use_markup(false)
            .build();
        style.add_suffix(&launch::mangohud_style_button());
        group.add(&style);
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

// ── Every block ─────────────────────────────────────────────────────────────

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
    /// Wine FSR, vkBasalt, AI Graphics.
    pub image_quality: ImageQualityFields,
    /// lsfg-vk.
    pub frame_generation: FrameGenFields,
    /// `MangoHud`.
    pub mangohud: MangoHudFields,
    live: Rc<Live>,
}

impl GameFields {
    /// Build every block from `g`. `titled` puts them in Tuning's sections,
    /// with their titles and what reaches the game (the editor); the wizard
    /// shows each block as a step of its own instead.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn build(g: &GameOptimization, m: &Machine, game: &Game, titled: bool) -> Self {
        let section = |title: &str, description: String| {
            titled.then(|| {
                let group = crate::widgets::optimization::section(&i18n(title));
                group.set_description(Some(&description));
                group
            })
        };
        let performance_group = section(
            "Performance",
            i18n(
                "Applied by falcond while this game runs, whichever launcher starts it. Saved through the privileged helper when you save.",
            ),
        );
        // Where the game's own Gamescope, Wine FSR and vkBasalt go.
        let reaches = match game.reach {
            Reach::Steam => i18n(
                "Written into this game's Steam launch options when you save, with Steam closed; your own options there are kept.",
            ),
            Reach::Launch => {
                i18n("For this game when BiGame-mode starts it (Profiles → Launch (Turbo)).")
            }
            Reach::Heroic { .. } => i18n(
                "Written into Heroic's settings for this game when you save, with Heroic closed; your own settings there are kept.",
            ),
            Reach::Unknown => i18n(
                "For this game when BiGame-mode starts it, and in its Steam launch options or its settings in Heroic when one of them starts it (written when you save, with that launcher closed).",
            ),
            Reach::Nothing => i18n(
                "Started through its own launcher, this game gets Tuning's settings from the session environment.",
            ),
        };
        let display_group = section(
            "Display",
            format!(
                "{reaches} {}",
                i18n("What is left on “General configuration” follows Tuning → Display.")
            ),
        );
        let image_group = section(
            "Image quality",
            format!(
                "{reaches} {}",
                i18n(
                    "What is left on “General configuration” comes from the session environment, as Tuning sets it."
                )
            ),
        );
        let frame_group = section(
            "Frame generation",
            i18n(
                "lsfg-vk reads this game's entry whichever launcher starts it; its Lossless.dll is one file for every game.",
            ),
        );
        let monitoring_group = section(
            "Monitoring",
            i18n(
                "Written when you save where this game's launcher reads it: Steam's launch options, its settings in Heroic or Lutris, or BiGame-mode's own launch.",
            ),
        );

        let live = Rc::new(Live::default());
        *live.launch.borrow_mut() = g.launch;
        live.mode.set(g.profile.gamescope_mode);

        let target = game.target.clone();
        let fields = Self {
            performance: PerformanceFields::build(g, m, performance_group.as_ref()),
            scheduler: SchedulerFields::build(g, m, performance_group.as_ref()),
            vcache: VCacheFields::build(g, m, performance_group.as_ref()),
            gamescope: GamescopeFields::build(m, game, &live, display_group.as_ref()),
            image_quality: ImageQualityFields::build(m, game, &live, image_group.as_ref()),
            frame_generation: FrameGenFields::build(
                g,
                m,
                game,
                frame_group.as_ref(),
                move |anchor| {
                    if let Some(target) = target.clone() {
                        crate::views::ai_graphics::open_to_change(anchor, target, |cfg| {
                            cfg.frame_generation =
                                bigame_core::graphics::config::FrameGeneration::Off;
                        });
                    }
                },
            ),
            mangohud: MangoHudFields::build(g, m, monitoring_group.as_ref()),
            live,
        };

        // Gamescope's upscaling and Wine FSR never run together in silence:
        // the change that would put both on for this game asks which one to
        // keep, and keeping the other puts the change back.
        let check: Check = {
            let l = Rc::downgrade(&fields.live);
            let general = m.video.upscaling.clone();
            let game = game.clone();
            Rc::new(move |before: Before, anchor: &gtk4::Widget| {
                let Some(live) = l.upgrade() else { return };
                let was = upscalers(&general, &game, before);
                let now = upscalers(&general, &game, live.snapshot());
                if !(now.0 && now.1) || (was.0 && was.1) {
                    return;
                }
                let (requested, active) = if now.1 && !was.1 {
                    (Feature::WineFsr, Feature::GamescopeUpscaling)
                } else {
                    (Feature::GamescopeUpscaling, Feature::WineFsr)
                };
                let Some(c) = opt::conflict(requested, active) else {
                    return;
                };
                let (l, anchor2) = (Rc::downgrade(&live), anchor.clone());
                live.asking.set(true);
                notice::ask_conflict(anchor, &c, move |use_requested| {
                    let Some(live) = l.upgrade() else { return };
                    live.asking.set(false);
                    if !use_requested {
                        live.restore(before);
                    } else if requested == Feature::WineFsr {
                        live.set(|l, _| l.render = Some((0, 0)));
                        crate::widgets::toast::show(
                            &anchor2,
                            &i18n("Wine FSR is on; Gamescope now renders at the game's own size"),
                        );
                    } else {
                        live.set(|l, _| l.wine_fsr = Some(false));
                        crate::widgets::toast::show(
                            &anchor2,
                            &i18n("Gamescope upscaling is on; Wine FSR was turned off"),
                        );
                    }
                });
            })
        };
        *fields.live.check.borrow_mut() = Some(check);
        fields
    }

    /// Write every block into `g`.
    pub fn apply(&self, g: &mut GameOptimization) {
        self.performance.apply(g);
        self.scheduler.apply(g);
        self.vcache.apply(g);
        let (launch, mode) = self.live.snapshot();
        g.launch = launch;
        g.profile.gamescope_mode = mode;
        self.frame_generation.apply(g);
        self.mangohud.apply(g);
    }
}
