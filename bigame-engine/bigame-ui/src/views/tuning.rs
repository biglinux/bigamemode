//! Tuning: the general configuration — what every game gets.
//!
//! The same six sections as a game's profile, in the same order and with
//! the same rows (`widgets::optimization`): Performance, Display, Image
//! quality, Frame generation, Monitoring, Advanced. A profile replaces what
//! it sets for its game; this page is everything else. Basic controls are
//! visible and the rest sits in expanders; what the machine cannot do is
//! said as *not supported* or *missing*, with the fix.
//!
//! Two kinds of settings live here, and each says which it is: falcond's
//! (written through the privileged helper, which reloads falcond) and the
//! launch settings in `video.toml` (read when BiGame-mode starts a game;
//! Wine FSR and vkBasalt also go into the session environment). Every
//! change is saved at once, off the main thread, one write at a time with
//! the newest state last (a spin button or a typed path waits until it
//! settles); a save that fails is said, with its reason, and the page goes
//! back to what is saved.
//!
//! Two technologies doing the same job are never left on together in
//! silence: switching one on while the other is on asks which to keep
//! (`widgets::notice::ask_conflict`), and a configuration that already has
//! both — from an older version — is named with the two ways out.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::models::{FrameGenBackend, GamescopeFilter};
use bigame_core::optimization::{self as opt, Feature, PROFILE_SETS, VCACHE_MODES};
use bigame_core::overview::State;
use bigame_core::video_config::{self, VideoConfig};

use crate::i18n::{N_, error_text, i18n, tr};
use crate::widgets::launch::{self, icon};
use crate::widgets::notice::{self, Kind, Notice};
use crate::widgets::optimization::{self as ui, Machine, Picker, Scope};
use crate::widgets::resolution::SizePicker;
use crate::widgets::status::Chip;

/// falcond's configuration, written through the helper.
type FalcondSaver = Rc<Saver<bigame_core::config::FalcondConfig>>;

/// The launch settings (`video.toml` and the session environment).
type VideoSaver = Rc<Saver<VideoConfig>>;

/// How long a spin button or a typed path must rest before it is saved.
const SETTLE_MS: u32 = 500;

/// Build the Tuning page. What it shows is read off the main thread (the
/// helper's file, sched-ext over D-Bus, `PATH`) and filled in when it comes.
#[must_use]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.add(&ui::scope_banner(Scope::General));
    let filled = page.clone();
    glib::spawn_future_local(async move {
        if let Ok(m) = gio::spawn_blocking(Machine::detect).await {
            fill(&filled, &m);
        }
    });
    page
}

fn fill(page: &adw::PreferencesPage, m: &Machine) {
    let falcond = Saver::new(
        m.general.clone(),
        |c| bigame_core::config::write_blocking(c).map(|()| None),
        N_("Could not save falcond's configuration"),
        None,
    );
    let video = Saver::new(
        m.video.clone(),
        video_config::save,
        N_("Could not save the launch settings"),
        Some(launch_settings_changed),
    );
    page.add(&build_performance(&falcond, m));
    let display = Display::build(&video, m);
    page.add(&display.group);
    page.add(&build_image_quality(&video, &display));
    page.add(&build_frame_generation(&video, m));
    page.add(&build_monitoring(m));
    page.add(&build_advanced(&falcond, m));
    // The savers hold the controls (to put them back) and the controls the
    // savers: let go when the page goes (Restore Defaults rebuilds it).
    let (falcond, video) = (Rc::downgrade(&falcond), Rc::downgrade(&video));
    page.connect_destroy(move |_| {
        if let Some(s) = falcond.upgrade() {
            s.release();
        }
        if let Some(s) = video.upgrade() {
            s.release();
        }
    });
}

/// What a toast says when the launch settings were saved but the running
/// session could not be brought to them.
#[must_use]
pub fn session_not_updated() -> String {
    i18n(
        "Saved, but the session environment could not be updated: games started now do not get the change until you log in again",
    )
}

/// One configuration's writes: at most one in flight, and whatever was
/// asked meanwhile written once it ends, so the newest state lands last.
#[derive(Debug, Default)]
struct WriteQueue {
    busy: bool,
    again: bool,
}

impl WriteQueue {
    /// A save is asked for: whether to start a write now (otherwise one is
    /// in flight, and another follows it).
    fn request(&mut self) -> bool {
        if self.busy {
            self.again = true;
            false
        } else {
            self.busy = true;
            true
        }
    }

    /// The write in flight ended: whether to start the one asked meanwhile.
    fn finished(&mut self) -> bool {
        if self.again {
            self.again = false;
            true
        } else {
            self.busy = false;
            false
        }
    }

    /// Forget what was asked while the write was in flight.
    fn cancel(&mut self) {
        self.again = false;
    }
}

/// How a configuration is written: `Ok(Some(_))` is saved, with a part
/// that did not take (the session environment).
type Write<T> = fn(&T) -> anyhow::Result<Option<anyhow::Error>>;

/// Puts one control back from a configuration.
type Resync<T> = Box<dyn Fn(&T)>;

/// Saves one configuration for the page, off the main thread.
///
/// A failed write puts the configuration and every control back to what was
/// last saved, so a later save never writes a change the user was told
/// failed.
struct Saver<T> {
    /// The configuration as the controls show it.
    shared: Rc<RefCell<T>>,
    /// What the file holds, as far as the page knows.
    saved: RefCell<T>,
    queue: RefCell<WriteQueue>,
    /// Bumped by every save asked; a delayed one runs only if none came
    /// after it.
    generation: Cell<u32>,
    /// Set while the controls are put back: their handlers save nothing.
    reverting: Cell<bool>,
    /// Each control, set from a configuration.
    resyncs: RefCell<Vec<Resync<T>>>,
    /// Run once the write that holds the change that asked for them is in.
    afters: RefCell<Vec<Box<dyn FnOnce()>>>,
    /// Where toasts go: the control last changed.
    anchor: RefCell<Option<gtk4::Widget>>,
    write: Write<T>,
    failed: &'static str,
    /// Run after every successful write.
    written: Option<fn(&gtk4::Widget)>,
}

impl<T: Clone + Send + 'static> Saver<T> {
    fn new(
        value: T,
        write: Write<T>,
        failed: &'static str,
        written: Option<fn(&gtk4::Widget)>,
    ) -> Rc<Self> {
        Rc::new(Self {
            shared: Rc::new(RefCell::new(value.clone())),
            saved: RefCell::new(value),
            queue: RefCell::default(),
            generation: Cell::new(0),
            reverting: Cell::new(false),
            resyncs: RefCell::default(),
            afters: RefCell::default(),
            anchor: RefCell::default(),
            write,
            failed,
            written,
        })
    }

    /// The configuration as the controls show it.
    fn shared(&self) -> &Rc<RefCell<T>> {
        &self.shared
    }

    /// How to put a control back after a failed save.
    fn on_revert(&self, f: impl Fn(&T) + 'static) {
        self.resyncs.borrow_mut().push(Box::new(f));
    }

    /// Save now.
    fn save(self: &Rc<Self>, anchor: &impl IsA<gtk4::Widget>) {
        self.request(anchor.upcast_ref(), 0, None);
    }

    /// Save once the control has rested for [`SETTLE_MS`].
    fn save_settled(self: &Rc<Self>, anchor: &impl IsA<gtk4::Widget>) {
        self.request(anchor.upcast_ref(), SETTLE_MS, None);
    }

    /// Save now, and run `after` once it is written.
    fn save_then(self: &Rc<Self>, anchor: &impl IsA<gtk4::Widget>, after: impl FnOnce() + 'static) {
        self.request(anchor.upcast_ref(), 0, Some(Box::new(after)));
    }

    fn request(
        self: &Rc<Self>,
        anchor: &gtk4::Widget,
        delay_ms: u32,
        after: Option<Box<dyn FnOnce()>>,
    ) {
        if self.reverting.get() {
            return;
        }
        *self.anchor.borrow_mut() = Some(anchor.clone());
        self.afters.borrow_mut().extend(after);
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        if delay_ms == 0 {
            self.kick();
            return;
        }
        let me = Rc::clone(self);
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(delay_ms.into()),
            move || {
                if me.generation.get() == generation {
                    me.kick();
                }
            },
        );
    }

    fn kick(self: &Rc<Self>) {
        if self.queue.borrow_mut().request() {
            self.start();
        }
    }

    /// Write the configuration as it is now; then the one asked meanwhile.
    fn start(self: &Rc<Self>) {
        let snapshot = self.shared.borrow().clone();
        let afters = std::mem::take(&mut *self.afters.borrow_mut());
        let write = self.write;
        let me = Rc::clone(self);
        glib::spawn_future_local(async move {
            let kept = snapshot.clone();
            let result = gio::spawn_blocking(move || write(&snapshot)).await;
            let anchor = me.anchor.borrow().clone();
            match result {
                Ok(Ok(partly)) => {
                    *me.saved.borrow_mut() = kept;
                    for after in afters {
                        after();
                    }
                    if let Some(anchor) = &anchor {
                        if let Some(e) = partly {
                            crate::widgets::toast::error(
                                anchor,
                                &session_not_updated(),
                                &error_text(&e),
                            );
                        }
                        if let Some(written) = me.written {
                            written(anchor);
                        }
                    }
                }
                failed => {
                    let why = match failed {
                        Ok(Err(e)) => error_text(&e),
                        _ => i18n("the worker thread stopped"),
                    };
                    if let Some(anchor) = &anchor {
                        crate::widgets::toast::error(anchor, &i18n(me.failed), &why);
                    }
                    me.revert();
                }
            }
            if me.queue.borrow_mut().finished() {
                me.start();
            }
        });
    }

    /// Let go of the controls, whose page is gone.
    fn release(&self) {
        self.resyncs.borrow_mut().clear();
        self.anchor.borrow_mut().take();
    }

    /// Put the configuration and the controls back to what is saved, and
    /// drop whatever was waiting to be written on top of the failed change.
    fn revert(&self) {
        self.queue.borrow_mut().cancel();
        self.generation.set(self.generation.get().wrapping_add(1));
        self.afters.borrow_mut().clear();
        let saved = self.saved.borrow().clone();
        *self.shared.borrow_mut() = saved.clone();
        self.reverting.set(true);
        for resync in self.resyncs.borrow().iter() {
            resync(&saved);
        }
        // A handler run by a control moving back may have copied another
        // control's state on the way; what is saved is what holds.
        *self.shared.borrow_mut() = saved;
        self.reverting.set(false);
    }
}

/// The launch settings changed (a save, Restore Defaults): bring the Steam
/// and Heroic games that take Tuning's values in line, once changes settle.
pub fn launch_settings_changed(anchor: &gtk4::Widget) {
    schedule_steam_gamescope(anchor);
}

thread_local! {
    /// Bumped by every change; a refresh runs only if no change came after
    /// the one that scheduled it.
    static STEAM_REFRESH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A Steam game whose profile says Gamescope Always takes Tuning's sizes and
/// filter in its launch options: after a change settles (a spin button
/// sends one per step), bring them in line, and say what happened.
fn schedule_steam_gamescope(anchor: &gtk4::Widget) {
    let generation = STEAM_REFRESH.with(|g| {
        g.set(g.get().wrapping_add(1));
        g.get()
    });
    let anchor = anchor.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
        if STEAM_REFRESH.with(std::cell::Cell::get) != generation {
            return;
        }
        glib::spawn_future_local(async move {
            // Heroic games too: what a game leaves to Tuning is written
            // into its settings there.
            let heroic = gio::spawn_blocking(bigame_core::optimization::refresh_heroic)
                .await
                .unwrap_or_default();
            report_heroic(&anchor, &heroic);
            let results = gio::spawn_blocking(bigame_core::optimization::refresh_steam_gamescope)
                .await
                .unwrap_or_default();
            report_steam_gamescope(&anchor, &results);
        });
    });
}

/// Say what bringing the Heroic games' settings in line with Tuning did; a
/// Heroic that is open and runs no game is offered to be closed for it.
fn report_heroic(
    anchor: &gtk4::Widget,
    results: &[(String, anyhow::Result<bigame_core::heroic_launch::Applied>)],
) {
    use bigame_core::heroic_launch::Applied;
    let written: Vec<&str> = results
        .iter()
        .filter(|(_, r)| matches!(r, Ok(Applied::Written)))
        .map(|(n, _)| n.as_str())
        .collect();
    if let Some((name, Err(e))) = results.iter().find(|(_, r)| r.is_err()) {
        crate::widgets::toast::error(
            anchor,
            &i18n("Could not update Heroic's settings for a game"),
            &format!("{name}: {}", error_text(e)),
        );
    }
    if !written.is_empty() {
        crate::widgets::toast::show(
            anchor,
            &i18n("Launch settings updated in Heroic's settings: %s")
                .replace("%s", &written.join(", ")),
        );
    }
    let open = results.iter().find_map(|(_, r)| match r {
        Ok(Applied::HeroicRunning {
            launcher,
            game_running,
        }) => Some((*launcher, *game_running)),
        _ => None,
    });
    match open {
        Some((_, true)) => crate::widgets::toast::show(
            anchor,
            &i18n(
                "Heroic is running a game: its games keep their old launch settings until Heroic is closed and Tuning is saved again",
            ),
        ),
        Some((launcher, false)) => crate::widgets::optimization::offer_close_heroic(
            anchor,
            launcher,
            &i18n(
                "Heroic is open: its games with launch settings of their own keep the old ones until it is closed",
            ),
            || {
                let results = bigame_core::optimization::refresh_heroic();
                match results
                    .into_iter()
                    .find_map(|(name, r)| r.err().map(|e| (name, e)))
                {
                    Some((name, e)) => Err(e.context(name)),
                    None => Ok(()),
                }
            },
        ),
        None => {}
    }
}

/// Say what bringing the Steam games' Gamescope wrappers in line did.
fn report_steam_gamescope(
    anchor: &gtk4::Widget,
    results: &[(
        String,
        anyhow::Result<bigame_core::steam_gamescope::Applied>,
    )],
) {
    use bigame_core::steam_gamescope::Applied;
    let written: Vec<&str> = results
        .iter()
        .filter(|(_, r)| matches!(r, Ok(Applied::Written(_))))
        .map(|(n, _)| n.as_str())
        .collect();
    // Games whose own launch options already run a Gamescope keep theirs.
    let theirs: Vec<&str> = results
        .iter()
        .filter(|(_, r)| matches!(r, Ok(Applied::TheirGamescope(_))))
        .map(|(n, _)| n.as_str())
        .collect();
    if !theirs.is_empty() {
        crate::widgets::toast::show(
            anchor,
            &i18n("These games' launch options already run a Gamescope of their own, so Big Game Mode's was not added: %s")
                .replace("%s", &theirs.join(", ")),
        );
    }
    let blocked = results
        .iter()
        .any(|(_, r)| matches!(r, Ok(Applied::SteamRunning)));
    if let Some((name, Err(e))) = results.iter().find(|(_, r)| r.is_err()) {
        crate::widgets::toast::error(
            anchor,
            &i18n("Could not update Gamescope in Steam's launch options"),
            &format!("{name}: {}", error_text(e)),
        );
    }
    if !written.is_empty() {
        crate::widgets::toast::show(
            anchor,
            &i18n("Gamescope updated in Steam's launch options: %s")
                .replace("%s", &written.join(", ")),
        );
    }
    if blocked {
        let a = anchor.clone();
        crate::widgets::toast::with_action(
            anchor,
            &i18n("Steam is open: its games with Gamescope keep the old sizes until it is closed"),
            &i18n("Close Steam and apply"),
            move || {
                let a = a.clone();
                glib::spawn_future_local(async move {
                    let done = gio::spawn_blocking(|| {
                        bigame_core::steam::while_closed(
                            bigame_core::optimization::refresh_steam_gamescope,
                        )
                    })
                    .await;
                    match done {
                        Ok(Ok(results)) => report_steam_gamescope(&a, &results),
                        Ok(Err(e)) => crate::widgets::toast::error(
                            &a,
                            &i18n("Steam could not be opened again"),
                            &error_text(&e),
                        ),
                        Err(_) => {}
                    }
                });
            },
        );
    }
}

// ── Performance (falcond) ───────────────────────────────────────────────────

/// Performance mode, the scheduler and 3D V-Cache, as falcond applies them.
#[allow(clippy::too_many_lines)]
fn build_performance(falcond: &FalcondSaver, m: &Machine) -> adw::PreferencesGroup {
    let shared = falcond.shared();
    let group = ui::section(&i18n("Performance"));
    group.set_description(Some(&i18n(
        "Applied by falcond while Turbo is on, and undone when it is turned off. Saved through the privileged helper.",
    )));

    let perf = adw::SwitchRow::builder()
        .title(i18n("Performance mode"))
        .subtitle(i18n(
            "The performance power profile while a game whose profile asks for it runs. Off, no game gets it.",
        ))
        .active(shared.borrow().enable_performance_mode)
        .build();
    group.add(&perf);
    {
        let me = Rc::clone(falcond);
        perf.connect_active_notify(move |row| {
            me.shared().borrow_mut().enable_performance_mode = row.is_active();
            me.save(row);
        });
        let perf = perf.clone();
        falcond.on_revert(move |c| perf.set_active(c.enable_performance_mode));
    }

    if let Some(row) = ui::scheduler_unavailable_row(m) {
        group.add(&row);
    } else {
        let sched = Picker::new(
            &i18n("CPU scheduler"),
            &i18n(
                "Loaded while Turbo is on. A game whose profile names another scheduler switches to it while it runs.",
            ),
            &ui::scheduler_items(m, Scope::General),
            &shared.borrow().scx_sched,
        );
        sched
            .row
            .add_suffix(&crate::widgets::scheduler_info::button());
        group.add(&sched.row);
        // What is loaded now, beside what is asked for: in the subtitle, so
        // the choice keeps its width. Read again after a save: the helper
        // restarts falcond, which loads the scheduler as it starts.
        let refresh_now: Rc<dyn Fn(u32)> = {
            let row = sched.row.clone();
            Rc::new(move |delay_ms: u32| {
                let row = row.clone();
                glib::spawn_future_local(async move {
                    glib::timeout_future(std::time::Duration::from_millis(delay_ms.into())).await;
                    let loaded = gio::spawn_blocking(bigame_core::running::loaded_scheduler)
                        .await
                        .ok()
                        .flatten();
                    let now = match loaded {
                        Some(name) => i18n("Now: %s").replace("%s", &name),
                        None => i18n("Now: the kernel's own scheduler"),
                    };
                    row.set_subtitle(&format!(
                        "{}\n{now}",
                        i18n(
                            "Loaded while Turbo is on. A game whose profile names another scheduler switches to it while it runs."
                        )
                    ));
                });
            })
        };
        refresh_now(0);

        let mode = Picker::new(
            &i18n("Scheduler mode"),
            "",
            &ui::mode_items(),
            &shared.borrow().scx_sched_props,
        );
        mode.row
            .add_suffix(&crate::widgets::info::scheduler_modes_button());
        mode.row
            .set_sensitive(!opt::inherits(&shared.borrow().scx_sched));
        group.add(&mode.row);
        {
            let (falcond, mode) = (Rc::clone(falcond), mode.clone());
            let row = sched.row.clone();
            let refresh = Rc::clone(&refresh_now);
            sched.connect_changed(move |value| {
                value.clone_into(&mut falcond.shared().borrow_mut().scx_sched);
                mode.row.set_sensitive(!opt::inherits(value));
                let refresh = Rc::clone(&refresh);
                falcond.save_then(&row, move || refresh(3000));
            });
        }
        {
            let falcond = Rc::clone(falcond);
            let row = mode.row.clone();
            let refresh = Rc::clone(&refresh_now);
            mode.connect_changed(move |value| {
                value.clone_into(&mut falcond.shared().borrow_mut().scx_sched_props);
                let refresh = Rc::clone(&refresh);
                falcond.save_then(&row, move || refresh(3000));
            });
        }
        falcond.on_revert(move |c| {
            sched.set(&c.scx_sched);
            mode.set(&c.scx_sched_props);
        });
    }

    if m.vcache {
        let items: Vec<(String, String)> = VCACHE_MODES
            .iter()
            .map(|c| (c.id.to_owned(), i18n(c.label)))
            .collect();
        let vcache = Picker::new(
            &i18n("3D V-Cache"),
            &i18n("Which CCD games prefer while Turbo is on"),
            &items,
            &shared.borrow().vcache_mode,
        );
        vcache
            .row
            .add_suffix(&crate::widgets::info::vcache_button());
        group.add(&vcache.row);
        {
            let falcond = Rc::clone(falcond);
            let row = vcache.row.clone();
            vcache.connect_changed(move |value| {
                value.clone_into(&mut falcond.shared().borrow_mut().vcache_mode);
                falcond.save(&row);
            });
        }
        falcond.on_revert(move |c| vcache.set(&c.vcache_mode));
    } else {
        group.add(&ui::vcache_unsupported_row());
    }
    group
}

// ── Display (Gamescope) ─────────────────────────────────────────────────────

/// Gamescope: on or off, the filter, sharpness and sizes — all in view.
struct Display {
    group: adw::PreferencesGroup,
    /// The Gamescope switch and the render size, for the conflict check;
    /// `None` without Gamescope.
    rows: Option<(adw::SwitchRow, Rc<SizePicker>)>,
    /// Set while the page itself changes a control, so a handler does not
    /// ask about a change the user did not make.
    quiet: Rc<Cell<bool>>,
}

impl Display {
    #[allow(clippy::too_many_lines)]
    fn build(saver: &VideoSaver, m: &Machine) -> Self {
        let video = saver.shared();
        let group = ui::section(&i18n("Display"));
        group.set_description(Some(&i18n(
            "For games started from Big Game Mode (Profiles → Launch (Turbo)). A game's profile can force Gamescope on or off, or set any of these values for itself.",
        )));
        let quiet = Rc::new(Cell::new(false));
        if !m.gamescope {
            group.add(&ui::missing_row(
                "Gamescope",
                &i18n("Not installed. It wraps the game in a micro-compositor: scaling, a frame limit, a stable fullscreen."),
                "sudo pacman -S gamescope",
            ));
            return Self {
                group,
                rows: None,
                quiet,
            };
        }
        let cfg = video.borrow().clone();
        let switch = adw::SwitchRow::builder()
            .title("Gamescope")
            .subtitle(i18n("Wraps games started from Big Game Mode"))
            .active(cfg.upscaling.gamescope_enabled)
            .build();
        switch.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
        // The version comes from `gamescope --help`, probed off the main thread.
        {
            let switch = switch.clone();
            glib::spawn_future_local(async move {
                let caps = gio::spawn_blocking(bigame_core::capabilities::gamescope_cached)
                    .await
                    .ok()
                    .flatten();
                if let Some(v) = caps.and_then(|c| c.version) {
                    switch.set_subtitle(
                        &i18n("Version %v · wraps games started from Big Game Mode")
                            .replace("%v", &v.to_string()),
                    );
                }
            });
        }
        group.add(&switch);

        let filter = launch::filter_picker(false, Some(cfg.upscaling.gamescope_filter));
        group.add(&filter.row);

        let sharpness_row = adw::SpinRow::new(
            Some(&gtk4::Adjustment::new(
                f64::from(cfg.upscaling.gamescope_sharpness.min(20)),
                0.0,
                20.0,
                1.0,
                5.0,
                0.0,
            )),
            1.0,
            0,
        );
        sharpness_row.set_title(&i18n("FSR sharpness"));
        show_sharpness(&sharpness_row, cfg.upscaling.gamescope_filter);
        group.add(&sharpness_row);

        let render = SizePicker::new(
            &i18n("Render size"),
            &i18n("The game draws at this size"),
            &i18n("The game's own size"),
            (cfg.upscaling.base_width, cfg.upscaling.base_height),
        );
        render.with_main_screen(false);
        group.add(&render.row);
        let output = SizePicker::new(
            &i18n("Output size"),
            &i18n("Upscaled to this size, usually the screen's"),
            &i18n("The same as the render size"),
            (cfg.upscaling.target_width, cfg.upscaling.target_height),
        );
        output.with_main_screen(true);
        group.add(&output.row);

        {
            let saver = Rc::clone(saver);
            let sharpness_row = sharpness_row.clone();
            let row = filter.row.clone();
            filter.connect_changed(move |id| {
                let f = launch::filter_of(id).unwrap_or_default();
                show_sharpness(&sharpness_row, f);
                saver.shared().borrow_mut().upscaling.gamescope_filter = f;
                saver.save(&row);
            });
        }
        {
            let saver = Rc::clone(saver);
            // One step at a time would be one write per step: saved once it rests.
            sharpness_row.connect_changed(move |row| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = row.value() as u8;
                saver.shared().borrow_mut().upscaling.gamescope_sharpness = v;
                saver.save_settled(row);
            });
        }
        {
            let saver = Rc::clone(saver);
            let row = output.row.clone();
            output.connect_changed(move |(w, h)| {
                {
                    let mut v = saver.shared().borrow_mut();
                    v.upscaling.target_width = w;
                    v.upscaling.target_height = h;
                }
                saver.save(&row);
            });
        }
        {
            let (switch, render, quiet) = (switch.clone(), Rc::clone(&render), Rc::clone(&quiet));
            saver.on_revert(move |c| {
                let u = &c.upscaling;
                quiet.set(true);
                switch.set_active(u.gamescope_enabled);
                render.set((u.base_width, u.base_height));
                quiet.set(false);
                filter.set(
                    ["fsr", "nis", "integer"]
                        .into_iter()
                        .find(|id| launch::filter_of(id) == Some(u.gamescope_filter))
                        .unwrap_or_default(),
                );
                sharpness_row.set_value(f64::from(u.gamescope_sharpness.min(20)));
                output.set((u.target_width, u.target_height));
            });
        }
        Self {
            group,
            rows: Some((switch, render)),
            quiet,
        }
    }
}

/// Gamescope's sharpness applies to its FSR filter only: with another, the
/// row is greyed out and says why.
fn show_sharpness(row: &adw::SpinRow, filter: GamescopeFilter) {
    let fsr = filter == GamescopeFilter::Fsr;
    row.set_sensitive(fsr);
    row.set_subtitle(&if fsr {
        i18n("0 = sharpest · 20 = softest")
    } else {
        i18n("Only for the FSR filter")
    });
}

// ── Image quality ───────────────────────────────────────────────────────────

/// Wine FSR and vkBasalt, with the conflict against Gamescope's upscaling
/// resolved in both directions.
#[allow(clippy::too_many_lines)]
fn build_image_quality(saver: &VideoSaver, display: &Display) -> adw::PreferencesGroup {
    let video = saver.shared();
    let group = ui::section(&i18n("Image quality"));
    group.set_description(Some(&i18n(
        "Set in the session environment for every game started afterwards. A launcher already running (Steam) keeps its old environment: close and reopen it.",
    )));
    let cfg = video.borrow().clone();

    // A configuration that already has both — an older version allowed it.
    let legacy = Notice::new(
        Kind::Conflict,
        &i18n("Wine FSR and Gamescope upscaling are both on"),
        &i18n(
            "Two upscalers in series scale the image twice. Until one is chosen, Wine FSR is switched off at launch for games Gamescope upscales.",
        ),
    );
    legacy.set_visible(!opt::general_conflicts(&cfg).is_empty());
    group.add(legacy.widget());

    let wine = adw::SwitchRow::builder()
        .title("Wine FSR")
        .subtitle(i18n(
            "Wine's own upscaling for Proton games in exclusive fullscreen (WINE_FULLSCREEN_FSR=1)",
        ))
        .active(cfg.upscaling.wine_fsr_enabled)
        .build();
    group.add(&wine);
    wine.add_prefix(&gtk4::Image::from_icon_name("zoom-in-symbolic"));
    let quality_picker = launch::wine_fsr_mode_picker(false, Some(cfg.upscaling.wine_fsr_mode));
    let quality = quality_picker.row.clone();
    quality.set_sensitive(cfg.upscaling.wine_fsr_enabled);
    group.add(&quality);
    {
        let saver = Rc::clone(saver);
        let row = quality.clone();
        quality_picker.connect_changed(move |id| {
            saver.shared().borrow_mut().upscaling.wine_fsr_mode =
                launch::wine_fsr_mode_of(id).unwrap_or_default();
            saver.save(&row);
        });
    }
    {
        let (wine, quiet) = (wine.clone(), Rc::clone(&display.quiet));
        saver.on_revert(move |c| {
            let u = &c.upscaling;
            quiet.set(true);
            wine.set_active(u.wine_fsr_enabled);
            quiet.set(false);
            quality_picker.set(
                ["performance", "balanced", "quality", "ultra"]
                    .into_iter()
                    .find(|id| launch::wine_fsr_mode_of(id) == Some(u.wine_fsr_mode))
                    .unwrap_or_default(),
            );
        });
    }

    let quiet = Rc::clone(&display.quiet);
    // Once resolved, the notice says what is on now instead of vanishing.
    let refresh_legacy = {
        let (legacy, video) = (legacy.clone(), Rc::clone(video));
        let had = Cell::new(!opt::general_conflicts(&cfg).is_empty());
        Rc::new(move || {
            let now = !opt::general_conflicts(&video.borrow()).is_empty();
            if had.get() && !now {
                let kept = if video.borrow().upscaling.wine_fsr_enabled {
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
            }
            had.set(now);
        })
    };

    // The page's own way to switch a feature off, used by both prompts and
    // by the legacy notice: the control moves, the file follows.
    let turn_off: Rc<dyn Fn(Feature)> = {
        let (saver, wine, quiet) = (Rc::clone(saver), wine.clone(), Rc::clone(&quiet));
        let rows = display.rows.clone();
        let refresh = Rc::clone(&refresh_legacy);
        Rc::new(move |f: Feature| {
            quiet.set(true);
            match f {
                Feature::WineFsr => wine.set_active(false),
                Feature::GamescopeUpscaling => {
                    if let Some((_, render)) = &rows {
                        render.set((0, 0));
                    }
                }
                _ => {}
            }
            quiet.set(false);
            opt::turn_off_general(&mut saver.shared().borrow_mut(), f);
            saver.save(&wine);
            refresh();
        })
    };
    {
        let turn_off = Rc::clone(&turn_off);
        legacy.add_action(
            &i18n("Keep %s").replace("%s", &notice::feature_name(Feature::GamescopeUpscaling)),
            false,
            move |_| {
                turn_off(Feature::WineFsr);
            },
        );
    }
    {
        let turn_off = Rc::clone(&turn_off);
        legacy.add_action(&i18n("Use %s").replace("%s", "Wine FSR"), true, move |_| {
            turn_off(Feature::GamescopeUpscaling);
        });
    }

    // Wine FSR switched on while Gamescope upscales: ask.
    {
        let (saver, quality, quiet) = (Rc::clone(saver), quality.clone(), Rc::clone(&quiet));
        let video = Rc::clone(saver.shared());
        let turn_off = Rc::clone(&turn_off);
        let refresh = Rc::clone(&refresh_legacy);
        wine.connect_active_notify(move |row| {
            let on = row.is_active();
            quality.set_sensitive(on);
            if quiet.get() {
                return;
            }
            let upscales = opt::gamescope_upscales(&video.borrow().upscaling);
            if on && upscales {
                if let Some(c) = opt::conflict(Feature::WineFsr, Feature::GamescopeUpscaling) {
                    let (turn_off, row2, quiet2) =
                        (Rc::clone(&turn_off), row.clone(), Rc::clone(&quiet));
                    let (video2, anchor) = (Rc::clone(&video), row.clone());
                    notice::ask_conflict(row, &c, move |use_wine| {
                        if use_wine {
                            video2.borrow_mut().upscaling.wine_fsr_enabled = true;
                            turn_off(Feature::GamescopeUpscaling);
                            crate::widgets::toast::show(
                                &anchor,
                                &i18n(
                                    "Wine FSR is on; Gamescope now renders at the game's own size",
                                ),
                            );
                        } else {
                            quiet2.set(true);
                            row2.set_active(false);
                            quiet2.set(false);
                        }
                    });
                    return;
                }
            }
            video.borrow_mut().upscaling.wine_fsr_enabled = on;
            saver.save(row);
            refresh();
        });
    }

    // Gamescope's upscaling switched on (the switch, or a render size) while
    // Wine FSR is on: ask. Keeping Wine FSR puts the render size back.
    if let Some((switch, render)) = display.rows.clone() {
        let ask = {
            let (saver, wine, quiet) = (Rc::clone(saver), wine.clone(), Rc::clone(&quiet));
            let video = Rc::clone(saver.shared());
            let turn_off = Rc::clone(&turn_off);
            let refresh = Rc::clone(&refresh_legacy);
            let (switch, render) = (switch.clone(), Rc::clone(&render));
            Rc::new(move |before: (bool, (u32, u32))| {
                let after = (switch.is_active(), render.value());
                {
                    let mut v = video.borrow_mut();
                    v.upscaling.gamescope_enabled = after.0;
                    (v.upscaling.base_width, v.upscaling.base_height) = after.1;
                }
                let was = before.0 && before.1.0 > 0 && before.1.1 > 0;
                let now = opt::gamescope_upscales(&video.borrow().upscaling);
                if !quiet.get() && now && !was && wine.is_active() {
                    if let Some(c) = opt::conflict(Feature::GamescopeUpscaling, Feature::WineFsr) {
                        let (turn_off, saver2, quiet2) =
                            (Rc::clone(&turn_off), Rc::clone(&saver), Rc::clone(&quiet));
                        let (switch2, render2) = (switch.clone(), Rc::clone(&render));
                        notice::ask_conflict(&switch, &c, move |use_gamescope| {
                            if use_gamescope {
                                turn_off(Feature::WineFsr);
                                crate::widgets::toast::show(
                                    &switch2,
                                    &i18n("Gamescope upscaling is on; Wine FSR was turned off"),
                                );
                            } else {
                                quiet2.set(true);
                                switch2.set_active(before.0);
                                render2.set(before.1);
                                quiet2.set(false);
                                {
                                    let mut v = saver2.shared().borrow_mut();
                                    v.upscaling.gamescope_enabled = before.0;
                                    (v.upscaling.base_width, v.upscaling.base_height) = before.1;
                                }
                                saver2.save(&switch2);
                            }
                        });
                        // Nothing is saved until one is chosen: the file never
                        // holds both, even while the question is open.
                        return;
                    }
                }
                saver.save(&switch);
                refresh();
            })
        };
        let last = Rc::new(Cell::new((
            cfg.upscaling.gamescope_enabled,
            (cfg.upscaling.base_width, cfg.upscaling.base_height),
        )));
        let track = {
            let (last, switch, render) = (Rc::clone(&last), switch.clone(), Rc::clone(&render));
            let ask = Rc::clone(&ask);
            Rc::new(move || {
                let before = last.get();
                ask(before);
                last.set((switch.is_active(), render.value()));
            })
        };
        {
            let track = Rc::clone(&track);
            switch.connect_active_notify(move |_| track());
        }
        render.connect_changed(move |_| track());
    }

    // vkBasalt: a look, not a speed-up; only where its layer is installed.
    if bigame_core::capabilities::vkbasalt_installed() {
        build_vkbasalt(saver, &group);
    } else {
        group.add(&ui::missing_row(
            "vkBasalt",
            &i18n(
                "Not installed. Visual filters (sharpening, colour) for Vulkan and Proton games.",
            ),
            "sudo pacman -S vkbasalt",
        ));
    }

    let ai = Notice::new(
        Kind::Info,
        &i18n("AI Graphics is per game"),
        &i18n(
            "Upscaling inside the game (FSR 4, XeSS, DLSS through OptiScaler) is set from the game's card in Profiles. A game with it launches without Wine FSR and without a Gamescope render size.",
        ),
    );
    group.add(ai.widget());
    group
}

/// vkBasalt's switch, its look and its file.
#[allow(clippy::too_many_lines)]
fn build_vkbasalt(saver: &VideoSaver, group: &adw::PreferencesGroup) {
    use bigame_core::vkbasalt::{self as vkb, Style, StyleState};
    let video = saver.shared();
    let cfg = video.borrow().clone();
    let switch = adw::SwitchRow::builder()
        .title("vkBasalt")
        .subtitle(i18n(
            "Visual filters (sharpening, colour) for Vulkan and Proton games. A look, not a speed-up: it costs a little GPU time. In the game, Home turns it on and off.",
        ))
        .active(cfg.upscaling.vkbasalt_enabled)
        .build();
    switch.add_prefix(&gtk4::Image::from_icon_name("image-x-generic-symbolic"));
    group.add(&switch);
    {
        let saver = Rc::clone(saver);
        switch.connect_active_notify(move |row| {
            saver.shared().borrow_mut().upscaling.vkbasalt_enabled = row.is_active();
            saver.save(row);
        });
    }

    let own_label = match vkb::current_style() {
        StyleState::Defaults => i18n("vkBasalt's default"),
        _ => i18n("My own file"),
    };
    let look = Picker::new(
        &i18n("Look"),
        &i18n(
            "Written to ~/.config/vkBasalt/vkBasalt.conf; your own file is kept and comes back with “My own file”.",
        ),
        &[
            ("own".to_owned(), own_label),
            ("cas".to_owned(), i18n("CAS sharpening")),
            ("nara".to_owned(), i18n("Nara Linux (ReShade)")),
        ],
        match vkb::current_style() {
            StyleState::Style(Style::Cas) => "cas",
            StyleState::Style(Style::NaraLinux) => "nara",
            _ => "own",
        },
    );
    look.row.set_subtitle_lines(3);
    look.row.add_suffix(&launch::vkbasalt_looks_button());
    group.add(&look.row);

    let shown = cfg
        .upscaling
        .vkbasalt_config_path
        .clone()
        .unwrap_or_else(|| {
            vkb::default_config().map_or_else(String::new, |p| p.to_string_lossy().into_owned())
        });
    let conf = adw::EntryRow::builder()
        .title(i18n("Configuration file"))
        .text(&shown)
        .build();
    conf.add_prefix(&gtk4::Image::from_icon_name("text-x-generic-symbolic"));
    group.add(&conf);
    let missing = Notice::new(
        Kind::Warning,
        &i18n("This file does not exist"),
        &i18n("vkBasalt then uses its own defaults."),
    );
    missing.set_visible(!shown.is_empty() && !std::path::Path::new(&shown).exists());
    group.add(missing.widget());
    let quiet = Rc::new(Cell::new(false));
    {
        let (saver, missing, quiet) = (Rc::clone(saver), missing.clone(), Rc::clone(&quiet));
        // Saved once typing rests: a half-typed path is never written.
        conf.connect_changed(move |row| {
            let text = row.text().to_string();
            missing.set_visible(!text.is_empty() && !std::path::Path::new(&text).exists());
            if quiet.get() {
                return;
            }
            saver.shared().borrow_mut().upscaling.vkbasalt_config_path =
                (!text.is_empty()).then_some(text);
            saver.save_settled(row);
        });
    }
    {
        let (switch, conf, quiet) = (switch.clone(), conf.clone(), Rc::clone(&quiet));
        saver.on_revert(move |c| {
            switch.set_active(c.upscaling.vkbasalt_enabled);
            quiet.set(true);
            conf.set_text(
                &c.upscaling.vkbasalt_config_path.clone().unwrap_or_else(|| {
                    vkb::default_config()
                        .map_or_else(String::new, |p| p.to_string_lossy().into_owned())
                }),
            );
            quiet.set(false);
        });
    }

    // A look is written to vkBasalt's own file, so that is the file used.
    let point_at_user_file = {
        let (saver, conf, quiet) = (Rc::clone(saver), conf.clone(), Rc::clone(&quiet));
        Rc::new(move || {
            let file = vkb::user_config();
            let path = file.exists().then(|| file.to_string_lossy().into_owned());
            quiet.set(true);
            conf.set_text(&path.clone().unwrap_or_else(|| {
                vkb::default_config().map_or_else(String::new, |p| p.to_string_lossy().into_owned())
            }));
            quiet.set(false);
            saver.shared().borrow_mut().upscaling.vkbasalt_config_path = path;
            saver.save(&conf);
        })
    };
    let previous = Rc::new(RefCell::new(look.value()));
    let picker = look.clone();
    // Set while a cancelled choice is put back: not a choice of its own.
    let reverting = Rc::new(Cell::new(false));
    let revert = {
        let (picker, previous, reverting) =
            (picker.clone(), Rc::clone(&previous), Rc::clone(&reverting));
        Rc::new(move || {
            reverting.set(true);
            picker.set(&previous.borrow());
            reverting.set(false);
        })
    };
    let apply = {
        let (row, previous) = (look.row.clone(), Rc::clone(&previous));
        let (point, revert) = (Rc::clone(&point_at_user_file), Rc::clone(&revert));
        Rc::new(move |style: Style, id: &str| match vkb::set_style(style) {
            Ok(()) => {
                id.clone_into(&mut previous.borrow_mut());
                point();
                crate::widgets::toast::show(
                    &row,
                    &i18n("Saved. It applies the next time a game starts."),
                );
            }
            Err(e) => {
                crate::widgets::toast::error(
                    &row,
                    &i18n("Could not change vkBasalt's file"),
                    &error_text(&e),
                );
                revert();
            }
        })
    };
    look.connect_changed(move |value| {
        if reverting.get() {
            return;
        }
        let style = match value {
            "cas" => Style::Cas,
            "nara" => Style::NaraLinux,
            _ => Style::Own,
        };
        if style != Style::NaraLinux || vkb::shaders_ready() {
            apply(style, value);
            return;
        }
        // The shaders first, and only when asked.
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Download the Nara Linux style's shaders?"))
            .body(i18n(
                "Five small files (about 30 KB) from GitHub: Colourfulness and FilmGrain2 from ReShade's shaders, FakeHDR from SweetFX, and the headers they include. Each is a fixed version, checked against its SHA-256 before it is used, and goes into ~/.local/share/reshade. Without them vkBasalt would stop the game, so the style is written only after they are in place.",
            ))
            .build();
        dialog.add_response("cancel", &i18n("Cancel"));
        dialog.add_response("download", &i18n("Download and use"));
        dialog.set_response_appearance("download", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("download"));
        dialog.set_close_response("cancel");
        let (apply, row, revert) = (Rc::clone(&apply), picker.row.clone(), Rc::clone(&revert));
        dialog.connect_response(None, move |_, response| {
            if response != "download" {
                revert();
                return;
            }
            let (apply, row, revert) = (Rc::clone(&apply), row.clone(), Rc::clone(&revert));
            row.set_sensitive(false);
            glib::spawn_future_local(async move {
                let done = gio::spawn_blocking(|| vkb::fetch_shaders(&vkb::shader_dir())).await;
                row.set_sensitive(true);
                match done {
                    Ok(Ok(())) => apply(Style::NaraLinux, "nara"),
                    Ok(Err(e)) => {
                        crate::widgets::toast::error(
                            &row,
                            &i18n("Could not download the shaders"),
                            &error_text(&e),
                        );
                        revert();
                    }
                    Err(_) => revert(),
                }
            });
        });
        dialog.present(Some(&picker.row));
    });
}

// ── Frame generation ────────────────────────────────────────────────────────

/// lsfg-vk's general switch and its DLL. Each game's own frame generation
/// is in its profile.
fn build_frame_generation(saver: &VideoSaver, m: &Machine) -> adw::PreferencesGroup {
    let video = saver.shared();
    let group = ui::section(&i18n("Frame generation"));
    if !m.lsfg {
        group.add(&ui::missing_row(
            "lsfg-vk",
            &i18n("Not installed. Generates extra frames between rendered ones (Lossless Scaling's method, as a Vulkan layer). It needs your own Lossless.dll."),
            "sudo pacman -S lsfg-vk",
        ));
        return group;
    }
    let switch = adw::SwitchRow::builder()
        .title(i18n("lsfg-vk for every game with an entry"))
        .subtitle(i18n(
            "Raises the presented frame rate, not the rendered one, and adds latency. Off sets every entry aside; on puts them back.",
        ))
        .active(opt::lsfg_general_on(&video.borrow()))
        .build();
    group.add(&switch);
    {
        let me = Rc::clone(saver);
        switch.connect_active_notify(move |row| {
            let on = row.is_active();
            let frame_gen = {
                let mut v = me.shared().borrow_mut();
                v.frame_gen.enabled = on;
                v.frame_gen.backend = if on {
                    FrameGenBackend::LsfgVk
                } else {
                    FrameGenBackend::None
                };
                v.frame_gen.clone()
            };
            // lsfg-vk's file follows only a switch that was saved.
            let anchor = row.clone();
            me.save_then(row, move || {
                if let Err(e) = bigame_core::fg::sync_global_enablement(&frame_gen) {
                    crate::widgets::toast::error(
                        &anchor,
                        &i18n("Could not update lsfg-vk's file"),
                        &error_text(&e),
                    );
                }
            });
        });
        let switch = switch.clone();
        saver.on_revert(move |c| switch.set_active(opt::lsfg_general_on(c)));
    }
    let dll_state = Notice::new(Kind::Warning, "", "");
    let show_dll = {
        let dll_state = dll_state.clone();
        move |ready: bool| {
            if ready {
                dll_state.set_visible(false);
            } else {
                dll_state.set(
                    Kind::Warning,
                    &i18n("lsfg-vk needs your Lossless.dll"),
                    &i18n("Without it lsfg-vk loads and generates nothing, so it is switched off at launch."),
                );
                dll_state.set_visible(true);
            }
        }
    };
    show_dll(m.lsfg_dll);
    group.add(&crate::widgets::fg_controls::dll_row(show_dll));
    group.add(dll_state.widget());
    let per_game = Notice::new(
        Kind::Info,
        &i18n("Each game's frame generation is in its profile"),
        &i18n(
            "Profiles → the game → Edit profile → Frame generation. A game whose AI Graphics has OptiScaler generating frames launches with lsfg-vk off.",
        ),
    );
    group.add(per_game.widget());
    group
}

// ── Monitoring ──────────────────────────────────────────────────────────────

/// `MangoHud`: shown per game (in the profile); how it looks, here.
fn build_monitoring(m: &Machine) -> adw::PreferencesGroup {
    use bigame_core::mangohud::{Style, StyleState};
    let group = ui::section(&i18n("Monitoring"));
    if !m.mangohud {
        group.add(&ui::missing_row(
            "MangoHud",
            &i18n("Not installed. The performance overlay; it also captures the frametimes Measure the difference uses."),
            "sudo pacman -S mangohud",
        ));
        return group;
    }
    let row = adw::ActionRow::builder()
        .title("MangoHud")
        .subtitle(i18n(
            "Installed. Shown per game, from its profile (Off, On, Forced); Details says whether it loaded in the running game.",
        ))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    let chip = Chip::new(State::Configured);
    chip.set(State::Configured, Some(&i18n("Per game")));
    row.add_suffix(chip.widget());
    group.add(&row);

    let state = bigame_core::mangohud::current_style();
    let own_label = match state {
        StyleState::Defaults => i18n("Default"),
        _ => i18n("My own file"),
    };
    let style = Picker::new(
        &i18n("Overlay style"),
        &i18n(
            "Written to ~/.config/MangoHud/MangoHud.conf; your own file is kept and comes back with “My own file”.",
        ),
        &[
            ("own".to_owned(), own_label),
            ("basic".to_owned(), i18n("Steam Deck — basic")),
            ("full".to_owned(), i18n("Steam Deck — full")),
        ],
        match state {
            StyleState::Style(Style::Basic) => "basic",
            StyleState::Style(Style::Full) => "full",
            _ => "own",
        },
    );
    style.row.set_subtitle_lines(3);
    style.row.add_suffix(&launch::mangohud_style_button());
    group.add(&style.row);
    let row = style.row.clone();
    style.connect_changed(move |value| {
        let chosen = match value {
            "basic" => Style::Basic,
            "full" => Style::Full,
            _ => Style::Own,
        };
        match bigame_core::mangohud::set_style(chosen) {
            Ok(()) => crate::widgets::toast::show(
                &row,
                &i18n("Saved. It applies the next time a game starts."),
            ),
            Err(e) => crate::widgets::toast::error(
                &row,
                &i18n("Could not change MangoHud's file"),
                &error_text(&e),
            ),
        }
    });
    group
}

// ── Advanced ────────────────────────────────────────────────────────────────

/// falcond's own settings and the facts for the person who knows what a
/// scheduler flag is: last, each with its ⓘ.
#[allow(clippy::too_many_lines)]
fn build_advanced(falcond: &FalcondSaver, m: &Machine) -> adw::PreferencesGroup {
    let shared = falcond.shared();
    let group = ui::section(&i18n("Advanced"));
    group.set_description(Some(&i18n(
        "For those who want to look under the hood. The defaults suit almost everyone: nothing here needs changing to play.",
    )));

    let poll_row = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(shared.borrow().poll_interval_ms),
            500.0,
            60_000.0,
            500.0,
            1000.0,
            0.0,
        )),
        500.0,
        0,
    );
    poll_row.set_title(&i18n("Scan interval (ms)"));
    poll_row.set_subtitle(&i18n(
        "How often falcond looks for a running game. Lower reacts sooner and costs more; the default is 9000.",
    ));
    icon(&poll_row, "view-refresh-symbolic");
    poll_row.add_suffix(&crate::widgets::info::button(
        &i18n("Scan interval"),
        &i18n(
            "falcond sees a game start at once, from the kernel's process events. The scan is its safety net: every so many milliseconds, once no process has started for that long, it reads the process list again, which catches a game whose name changes after it starts. 9000 ms (9 s) is falcond's default; lower costs a little CPU for nothing on most machines.",
        ),
    ));
    group.add(&poll_row);
    {
        let me = Rc::clone(falcond);
        // Each step would be a privileged write and a falcond reload: saved
        // once the value rests.
        poll_row.connect_changed(move |row| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let val = row.value() as u32;
            me.shared().borrow_mut().poll_interval_ms = val;
            me.save_settled(row);
        });
        let poll_row = poll_row.clone();
        falcond.on_revert(move |c| poll_row.set_value(f64::from(c.poll_interval_ms)));
    }
    // The set's own name in the list; what it is for, under it.
    let sets: Vec<(String, String)> = PROFILE_SETS
        .iter()
        .map(|c| (c.id.to_owned(), i18n(c.label)))
        .collect();
    let help = |id: &str| {
        PROFILE_SETS
            .iter()
            .find(|c| c.id == id)
            .map_or_else(String::new, |c| i18n(c.help))
    };
    let set = Picker::new(
        &i18n("Profile set"),
        &help(&shared.borrow().profile_mode),
        &sets,
        &shared.borrow().profile_mode,
    );
    set.row
        .add_prefix(&gtk4::Image::from_icon_name("view-list-symbolic"));
    set.row
        .add_suffix(&crate::widgets::info::profile_sets_button());
    group.add(&set.row);
    {
        let falcond = Rc::clone(falcond);
        let row = set.row.clone();
        set.connect_changed(move |value| {
            row.set_subtitle(&help(value));
            value.clone_into(&mut falcond.shared().borrow_mut().profile_mode);
            falcond.save(&row);
        });
    }
    falcond.on_revert(move |c| set.set(&c.profile_mode));
    // The governor, for reference: power-profiles-daemon sets it.
    let gov_row = adw::ActionRow::builder()
        .title(i18n("CPU governor"))
        .subtitle(i18n("Reading…"))
        .use_markup(false)
        .build();
    icon(&gov_row, "speedometer-symbolic");
    gov_row.add_suffix(&crate::widgets::info::button(
        &i18n("CPU governor"),
        &i18n(
            "The kernel's rule for the CPU's clock. Big Game Mode does not set it: power-profiles-daemon does, from the power profile. Performance mode (above) asks for the performance profile while a game runs, and it goes back afterwards.",
        ),
    ));
    group.add(&gov_row);
    {
        let row = gov_row.clone();
        glib::spawn_future_local(async move {
            let (available, current) = gio::spawn_blocking(|| {
                let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
                (
                    read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors"),
                    read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                )
            })
            .await
            .unwrap_or_default();
            let current = current.trim();
            row.set_subtitle(&if current.is_empty() {
                i18n("No cpufreq driver reports a governor")
            } else {
                i18n("%c now, set by power-profiles-daemon through the power profile (available: %a)")
                    .replace("%c", current)
                    .replace("%a", available.trim())
            });
        });
    }

    let scx_status = adw::ActionRow::builder()
        .title(i18n("sched-ext availability"))
        .subtitle(match m.sched.describe_text() {
            Some(reason) => tr(&reason),
            None => i18n("Available — falcond applies the scheduler you configure above"),
        })
        .use_markup(false)
        .build();
    scx_status.add_prefix(&gtk4::Image::from_icon_name(if m.sched.is_available() {
        "object-select-symbolic"
    } else {
        "dialog-warning-symbolic"
    }));
    scx_status.add_suffix(&crate::widgets::info::button(
        &i18n("sched-ext"),
        &i18n(
            "sched-ext lets a CPU scheduler run as a program the kernel loads (scx_lavd, scx_bpfland…), instead of the kernel's own. It needs a kernel built with it (6.12 or later) and the scx-scheds package. The scheduler chosen in Performance is loaded while Turbo is on.",
        ),
    ));
    group.add(&scx_status);
    let installed = adw::ActionRow::builder()
        .title(i18n("Installed"))
        .subtitle(if m.sched_installed.is_empty() {
            i18n("none")
        } else {
            m.sched_installed.join(", ")
        })
        .use_markup(false)
        .build();
    icon(&installed, "application-x-addon-symbolic");
    group.add(&installed);
    let env_row = adw::ActionRow::builder()
        .title(i18n("Environment file"))
        .subtitle(i18n(
            "Wine FSR and vkBasalt are written to ~/.config/environment.d/bigame-mode.conf and pushed into the user systemd manager, so every game started afterwards inherits them.",
        ))
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    icon(&env_row, "text-x-generic-symbolic");
    env_row.add_suffix(&crate::widgets::info::button(
        &i18n("Environment file"),
        &i18n(
            "A game reads switches such as WINE_FULLSCREEN_FSR and ENABLE_VKBASALT from its environment, which it gets from whatever starts it. The file is read at login; the systemd user manager is updated at once, so a launcher opened afterwards passes them on. A launcher already open (Steam) keeps its old environment until it is reopened. A Turbo preset adds its own switches only while it is in force.",
        ),
    ));
    group.add(&env_row);

    // Gamescope's options come from `gamescope --help`, off the main thread.
    let gamescope_row = adw::ActionRow::builder()
        .title(i18n("Gamescope options this build accepts"))
        .subtitle(if m.gamescope {
            i18n("Reading…")
        } else {
            i18n("Gamescope is not installed")
        })
        .use_markup(false)
        .build();
    icon(&gamescope_row, "video-display-symbolic");
    group.add(&gamescope_row);
    if m.gamescope {
        let group = group.clone();
        glib::spawn_future_local(async move {
            let Some(gs) = gio::spawn_blocking(bigame_core::capabilities::gamescope_cached)
                .await
                .ok()
                .flatten()
            else {
                gamescope_row.set_subtitle(&i18n("Gamescope is not installed"));
                return;
            };
            gamescope_row.set_subtitle(
                &i18n("Version %v — %n options detected from --help")
                    .replace(
                        "%v",
                        &gs.version
                            .map_or_else(|| i18n("unknown"), |v| v.to_string()),
                    )
                    .replace("%n", &gs.flags.len().to_string()),
            );
            // The generated command line is the honest "advanced options"
            // box: the arguments come from capabilities.
            let sample = bigame_core::gamescope::Config {
                render_width: 1920,
                render_height: 1080,
                filter: bigame_core::gamescope::Filter::Fsr,
                sharpness: 5,
                ..bigame_core::gamescope::Config::default()
            };
            let built = sample.to_args(&gs);
            let preview = adw::ActionRow::builder()
                .title(i18n("Example command line"))
                .use_markup(false)
                .build();
            preview.set_subtitle(&format!("gamescope {} -- <game>", built.args.join(" ")));
            preview.set_subtitle_selectable(true);
            preview.add_prefix(&gtk4::Image::from_icon_name("utilities-terminal-symbolic"));
            group.add(&preview);
            for unsupported in &built.unsupported {
                let row = adw::ActionRow::builder()
                    .title(i18n("Not supported by this Gamescope"))
                    .subtitle(format!(
                        "--{} — {}",
                        unsupported.flag,
                        i18n(unsupported.effect)
                    ))
                    .use_markup(false)
                    .build();
                row.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
                group.add(&row);
            }
        });
    }
    group
}

#[cfg(test)]
mod tests {
    use super::WriteQueue;

    #[test]
    fn writes_never_overlap_and_the_newest_state_goes_last() {
        let mut q = WriteQueue::default();
        assert!(q.request(), "nothing in flight: write now");
        // Holding "+" on a spin button: one more write, not one per step.
        assert!(!q.request());
        assert!(!q.request());
        assert!(q.finished(), "what was asked meanwhile is written next");
        assert!(!q.finished(), "and then nothing is left");
        assert!(q.request());
    }

    #[test]
    fn a_failed_write_drops_what_was_queued_on_top_of_it() {
        let mut q = WriteQueue::default();
        assert!(q.request());
        assert!(!q.request());
        q.cancel();
        assert!(!q.finished());
        assert!(q.request(), "the next change starts a write of its own");
    }
}
