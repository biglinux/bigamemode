//! Problems: everything that needs attention, classified by what can be
//! done about it, and the full list of checks behind it.
//!
//! Two sources: the runtime findings of the snapshot (configured but not
//! detected, asked for but missing) and the system health checks
//! (`bigame_core::health`). Each is classed *fixable* (a command to copy),
//! *needs you* (advice), *hardware* (a limit, not a fault) or
//! *information*. Nothing here runs a command: a fix that needs root is
//! the user's to run.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::health::{Check, Fix, Status};
use bigame_core::overview::{Snapshot, State};

use crate::i18n::{i18n, ni18n, tr};

/// What can be done about a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A command fixes it.
    Fixable,
    /// Only the user can decide or act.
    NeedsYou,
    /// A hardware or kernel limit.
    Hardware,
    /// Worth knowing, nothing to do.
    Info,
    /// Fine.
    Ok,
}

impl Class {
    fn label(self) -> String {
        match self {
            Self::Fixable => i18n("Fixable"),
            Self::NeedsYou => i18n("Needs you"),
            Self::Hardware => i18n("Hardware"),
            Self::Info => i18n("Information"),
            Self::Ok => i18n("OK"),
        }
    }

    fn css(self) -> &'static str {
        match self {
            Self::Fixable | Self::NeedsYou => "state-attention",
            Self::Hardware | Self::Info => "state-neutral",
            Self::Ok => "state-active",
        }
    }
}

/// The class of a health check.
fn class_of(c: &Check) -> Class {
    match (c.status, &c.fix) {
        (Status::Ok, _) => Class::Ok,
        (Status::NotApplicable, _) => Class::Hardware,
        (Status::Info, _) => Class::Info,
        (_, Some(Fix::Command(_))) => Class::Fixable,
        (_, _) => Class::NeedsYou,
    }
}

/// The problems group.
#[derive(Clone)]
pub struct Problems {
    group: adw::PreferencesGroup,
    summary: adw::ActionRow,
    /// Rows from the snapshot, replaced on every reading.
    runtime_rows: Rc<RefCell<Vec<gtk4::Widget>>>,
    /// Rows from the health checks, replaced on every slow reading.
    health_rows: Rc<RefCell<Vec<gtk4::Widget>>>,
    /// The expander that holds the checks that are fine.
    fine: adw::ExpanderRow,
    fine_rows: Rc<RefCell<Vec<gtk4::Widget>>>,
    /// Text of every check, for the copy button.
    text: Rc<RefCell<String>>,
    counts: Rc<RefCell<(usize, usize)>>,
}

impl Problems {
    /// Build the group.
    #[must_use]
    pub fn new() -> Self {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Problems"));
        group.set_description(Some(&i18n(
            "What needs attention, and what can be done about it. Commands are copied, never run.",
        )));
        let copy_all = gtk4::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(i18n("Copy all checks"))
            .css_classes(["flat"])
            .build();
        copy_all.update_property(&[gtk4::accessible::Property::Label(&i18n("Copy all checks"))]);
        let text = Rc::new(RefCell::new(String::new()));
        {
            let text = Rc::clone(&text);
            copy_all.connect_clicked(move |b| {
                b.clipboard().set_text(&text.borrow());
                crate::widgets::toast::show(b, &i18n("Copied"));
            });
        }
        group.set_header_suffix(Some(&copy_all));

        let summary = adw::ActionRow::builder()
            .title(i18n("Checking…"))
            .use_markup(false)
            .build();
        let summary_icon = gtk4::Image::from_icon_name("emblem-synchronizing-symbolic");
        summary.add_prefix(&summary_icon);
        group.add(&summary);

        let fine = adw::ExpanderRow::builder()
            .title(i18n("Checks that passed"))
            .use_markup(false)
            .build();
        group.add(&fine);

        Self {
            group,
            summary,
            runtime_rows: Rc::new(RefCell::new(Vec::new())),
            health_rows: Rc::new(RefCell::new(Vec::new())),
            fine,
            fine_rows: Rc::new(RefCell::new(Vec::new())),
            text,
            counts: Rc::new(RefCell::new((0, 0))),
        }
    }

    /// The group.
    #[must_use]
    pub fn group(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    fn update_summary(&self) {
        let (runtime, health) = *self.counts.borrow();
        let total = runtime + health;
        let (icon, title) = if total == 0 {
            ("emblem-ok-symbolic", i18n("Nothing needs attention"))
        } else {
            (
                "dialog-warning-symbolic",
                ni18n("%n problem found", "%n problems found", total),
            )
        };
        self.summary.set_title(&title);
        if let Some(img) = self
            .summary
            .first_child()
            .and_then(|c| c.first_child())
            .and_then(|c| c.first_child())
            .and_downcast::<gtk4::Image>()
        {
            img.set_icon_name(Some(icon));
        }
    }

    /// Show the runtime findings of a reading: configured but not seen,
    /// asked for but missing.
    // One finding per item, in the page's order; a table would hide the
    // wording each one needs.
    #[allow(clippy::too_many_lines)]
    pub fn show_runtime(&self, snap: &Snapshot) {
        for w in self.runtime_rows.borrow_mut().drain(..) {
            self.group.remove(&w);
        }
        let game = snap.game.is_some();
        let mut findings: Vec<(String, String, Class, Option<String>)> = Vec::new();
        let mut add = |state: State, title: &str, detail: String, command: Option<&str>| {
            if !state.needs_attention() {
                return;
            }
            let class = if command.is_some() {
                Class::Fixable
            } else {
                Class::NeedsYou
            };
            findings.push((title.to_owned(), detail, class, command.map(str::to_owned)));
        };
        add(
            snap.turbo_state(),
            &i18n("Turbo"),
            match snap.turbo_state() {
                State::Error => {
                    i18n("falcond's service failed; turn Turbo off and on again, and see Logs.")
                }
                _ => i18n("falcond is not installed, so there is no per-game optimization."),
            },
            (snap.turbo_state() == State::Missing)
                .then_some("sudo pacman -S falcond falcond-profiles"),
        );
        if snap.turbo_state() == State::Active {
            add(
                snap.falcond_state(),
                "falcond",
                i18n(
                    "The service runs but its status file cannot be read; nothing it does can be verified.",
                ),
                None,
            );
        }
        add(
            snap.power_state(),
            &i18n("Power profile"),
            i18n(
                "A game runs under Turbo, but the power profile is not performance. Check the game's profile has performance mode on.",
            ),
            None,
        );
        let sched = snap.scheduler.state(game);
        add(
            sched,
            &i18n("CPU scheduler"),
            match sched {
                State::Missing => i18n(
                    "The profile asks for a sched-ext scheduler, but it cannot be switched on this machine.",
                ),
                _ => i18n(
                    "The scheduler the profile asked for is not the one running. See the Performance row for what falcond and the kernel report.",
                ),
            },
            match snap.scheduler.caps.switchable() {
                bigame_core::capabilities::Support::NotInstalled(p) if p == "scx-tools" => {
                    Some("sudo pacman -S scx-tools && sudo systemctl enable --now scx_loader")
                }
                bigame_core::capabilities::Support::NotInstalled(_) => {
                    Some("sudo pacman -S scx-scheds scx-tools")
                }
                bigame_core::capabilities::Support::ServiceDown(_) => {
                    Some("sudo systemctl enable --now scx_loader")
                }
                _ => None,
            },
        );
        add(
            snap.vcache.state(game),
            &i18n("3D V-Cache"),
            i18n("The mode the profile asked for is not the one set."),
            None,
        );
        add(
            snap.gamescope_state(),
            "Gamescope",
            match snap.gamescope_state() {
                State::Missing => i18n("Configured, but not installed."),
                _ => i18n(
                    "Configured, but the game is not running inside it. Start the game from Profiles → Launch (Turbo); a Steam game is started by Steam and gets Gamescope only through Steam's launch options.",
                ),
            },
            (snap.gamescope_state() == State::Missing).then_some("sudo pacman -S gamescope"),
        );
        add(
            snap.wine_fsr_state(),
            "Wine FSR",
            i18n(
                "Configured, but the variable is not in the game's environment. Close and reopen Steam, then start the game again.",
            ),
            None,
        );
        add(
            snap.vkbasalt_state(),
            "vkBasalt",
            match snap.vkbasalt_state() {
                State::Missing => i18n("Configured, but its Vulkan layer is not installed."),
                _ => i18n(
                    "Configured, but not loaded in the game: it loads in Vulkan and Proton games whose environment has ENABLE_VKBASALT=1.",
                ),
            },
            (snap.vkbasalt_state() == State::Missing).then_some("sudo pacman -S vkbasalt"),
        );
        add(
            snap.frame_generation_state(),
            &i18n("Frame generation"),
            match snap.frame_generation_state() {
                State::Missing if !snap.lsfg.installed => {
                    i18n("Configured, but lsfg-vk is not installed.")
                }
                State::Missing => i18n(
                    "Configured, but no Lossless.dll is set: set its path in Tuning → Frame generation.",
                ),
                _ => i18n(
                    "Configured, but not generating in the game. The layer generates only for a game that started with an entry; start it again.",
                ),
            },
            (snap.frame_generation_state() == State::Missing && !snap.lsfg.installed)
                .then_some("sudo pacman -S lsfg-vk"),
        );
        add(
            snap.mangohud_state(),
            "MangoHud",
            match snap.mangohud_state() {
                State::Missing => i18n("Chosen for this game, but not installed."),
                _ => i18n("Chosen for this game, but not loaded in it."),
            },
            (snap.mangohud_state() == State::Missing).then_some("sudo pacman -S mangohud"),
        );

        self.counts.borrow_mut().0 = findings.len();
        for (title, detail, class, command) in findings {
            let row = problem_row(&title, &detail, class, command.as_deref());
            self.group.add(&row);
            self.runtime_rows.borrow_mut().push(row.upcast());
        }
        // Runtime findings first, the health checks after them, the passed
        // ones last: re-append what was already there.
        for w in self.health_rows.borrow().iter() {
            self.group.remove(w);
            self.group.add(w);
        }
        self.group.remove(&self.fine);
        self.group.add(&self.fine);
        self.update_summary();
    }

    /// Run the health checks again, off the main thread, and show them.
    pub fn refresh_health(&self) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let checks = gio::spawn_blocking(bigame_core::health::collect)
                .await
                .unwrap_or_default();
            for w in this.health_rows.borrow_mut().drain(..) {
                this.group.remove(&w);
            }
            for w in this.fine_rows.borrow_mut().drain(..) {
                this.fine.remove(&w);
            }
            let mut all = String::new();
            let mut problems = 0;
            let mut fine = 0;
            for c in &checks {
                let class = class_of(c);
                let _ = writeln!(
                    all,
                    "{:?}\t{}\t{}{}",
                    c.status,
                    tr(&c.title),
                    tr(&c.detail),
                    c.fix
                        .as_ref()
                        .map_or_else(String::new, |f| format!("\t→ {}", fix_text(f)))
                );
                let command = match &c.fix {
                    Some(Fix::Command(cmd)) => Some(cmd.as_str()),
                    _ => None,
                };
                let detail = match &c.fix {
                    Some(Fix::Advice(a)) => format!("{} — {}", tr(&c.detail), tr(a)),
                    _ => tr(&c.detail),
                };
                let row = problem_row(&tr(&c.title), &detail, class, command);
                match class {
                    Class::Ok => {
                        this.fine.add_row(&row);
                        this.fine_rows.borrow_mut().push(row.upcast());
                        fine += 1;
                    }
                    Class::Fixable | Class::NeedsYou => {
                        this.group.add(&row);
                        this.health_rows.borrow_mut().push(row.upcast());
                        problems += 1;
                    }
                    Class::Hardware | Class::Info => {
                        this.group.add(&row);
                        this.health_rows.borrow_mut().push(row.upcast());
                    }
                }
            }
            // The passed checks go last, after whatever was just added.
            this.group.remove(&this.fine);
            this.group.add(&this.fine);
            *this.text.borrow_mut() = all;
            this.fine
                .set_title(&ni18n("%n check passed", "%n checks passed", fine));
            this.fine.set_visible(fine > 0);
            this.counts.borrow_mut().1 = problems;
            this.update_summary();
        });
    }
}

/// A fix as shown: advice translated, a command as it is.
fn fix_text(fix: &Fix) -> String {
    match fix {
        Fix::Command(c) => c.clone(),
        Fix::Advice(t) => tr(t),
    }
}

/// One finding: title, what was found, its class, and a copy button when a
/// command fixes it.
fn problem_row(title: &str, detail: &str, class: Class, command: Option<&str>) -> adw::ActionRow {
    let subtitle = match command {
        Some(c) => format!("{detail}\n→ {c}"),
        None => detail.to_owned(),
    };
    // Plain text: a command with `&&` is invalid Pango markup.
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(&subtitle)
        .subtitle_lines(6)
        .use_markup(false)
        .build();
    let icon = gtk4::Image::from_icon_name(match class {
        Class::Ok => "object-select-symbolic",
        Class::Fixable => "wrench-wide-symbolic",
        Class::NeedsYou => "dialog-warning-symbolic",
        Class::Hardware => "action-unavailable-symbolic",
        Class::Info => "dialog-information-symbolic",
    });
    icon.add_css_class(match class {
        Class::Ok => "success",
        Class::Fixable | Class::NeedsYou => "warning",
        Class::Hardware | Class::Info => "dim-label",
    });
    row.add_prefix(&icon);
    let badge = gtk4::Label::builder()
        .label(class.label())
        .css_classes(["caption", "state-chip", class.css()])
        .valign(gtk4::Align::Center)
        .build();
    row.add_suffix(&badge);
    if let Some(cmd) = command {
        let copy = gtk4::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(i18n("Copy the command"))
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        copy.update_property(&[gtk4::accessible::Property::Label(&i18n("Copy the command"))]);
        let cmd = cmd.to_owned();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&cmd);
            crate::widgets::toast::show(b, &i18n("Copied"));
        });
        row.add_suffix(&copy);
    }
    row
}
