//! The top of Details: one line on how the machine stands, and a grid of
//! state chips — the answer to "is it working?" before any scrolling.

use std::collections::HashMap;

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::overview::{AppliedProfile, Headline, Snapshot, State};

use crate::i18n::{i18n, ni18n};
use crate::widgets::status::Chip;

/// The overview group.
#[derive(Clone)]
pub struct Overview {
    group: adw::PreferencesGroup,
    icon: gtk4::Image,
    title: gtk4::Label,
    subtitle: gtk4::Label,
    chips: Vec<(&'static str, Chip)>,
    /// GPU display names by DRM card, read once.
    gpu_names: HashMap<String, String>,
    /// The card games are expected to render on.
    expected_gpu: Option<String>,
}

/// The chips, in order. Keys are stable ids, not labels.
const CHIPS: &[(&str, &str)] = &[
    ("turbo", "Turbo"),
    ("falcond", "falcond"),
    ("profile", "Profile"),
    ("power", "Power"),
    ("scheduler", "Scheduler"),
    ("gpu", "GPU"),
    ("gamescope", "Gamescope"),
    ("upscaling", "Upscaling"),
    ("framegen", "Frame generation"),
];

impl Overview {
    /// Build the group.
    #[must_use]
    pub fn new(hw: &bigame_core::hardware::Hardware) -> Self {
        let group = adw::PreferencesGroup::new();

        let icon = gtk4::Image::from_icon_name("emblem-synchronizing-symbolic");
        icon.set_pixel_size(28);
        icon.add_css_class("overview-icon");
        let title = gtk4::Label::builder()
            .label(i18n("Reading the system…"))
            .css_classes(["title-2"])
            .xalign(0.0)
            .wrap(true)
            .build();
        let subtitle = gtk4::Label::builder()
            .css_classes(["dim-label"])
            .xalign(0.0)
            .wrap(true)
            .build();
        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk4::Align::Center);
        text.append(&title);
        text.append(&subtitle);

        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        head.append(&icon);
        head.append(&text);

        let grid = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(true)
            .column_spacing(8)
            .row_spacing(8)
            .min_children_per_line(2)
            .max_children_per_line(5)
            .margin_top(12)
            .build();
        let mut chips = Vec::new();
        for (id, name) in CHIPS {
            let chip = Chip::new(State::Off);
            let caption = gtk4::Label::builder()
                .label(i18n(name))
                .css_classes(["caption", "dim-label"])
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            let cell = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            cell.add_css_class("overview-tile");
            cell.append(&caption);
            cell.append(chip.widget());
            grid.insert(&cell, -1);
            chips.push((*id, chip));
        }

        let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        card.add_css_class("card");
        card.add_css_class("overview-card");
        card.append(&head);
        card.append(&grid);
        group.add(&card);

        let (infos, expected) = bigame_core::graphics::report::gpu_infos(hw, None);
        let expected_gpu = expected.and_then(|i| infos.get(i)).map(|g| g.card.clone());
        let gpu_names = infos
            .into_iter()
            .map(|g| (g.card, bigame_core::graphics::report::display_name(&g.name)))
            .collect();

        Self {
            group,
            icon,
            title,
            subtitle,
            chips,
            gpu_names,
            expected_gpu,
        }
    }

    /// The group.
    #[must_use]
    pub fn group(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    fn chip(&self, id: &str) -> Option<&Chip> {
        self.chips.iter().find(|(k, _)| *k == id).map(|(_, c)| c)
    }

    /// Show a reading.
    // One pass over the chips, top to bottom; splitting it would scatter
    // the one place that maps the snapshot to the page.
    #[allow(clippy::too_many_lines)]
    pub fn show(&self, snap: &Snapshot) {
        let game_name = snap.game.as_ref().map(|g| g.display_name.clone());
        let (icon, title, mut subtitle) = match snap.headline() {
            Headline::TurboOff => (
                "media-playback-stop-symbolic",
                i18n("Turbo is off"),
                i18n("Games run without BiGame-mode's optimizations. Turn Turbo on at Home."),
            ),
            Headline::ReadyWaiting => (
                "emblem-ok-symbolic",
                i18n("Ready to play"),
                i18n("Turbo is on. The next game gets its profile as it starts."),
            ),
            Headline::Optimizing => (
                "emblem-ok-symbolic",
                i18n("Optimizing %s").replace("%s", game_name.as_deref().unwrap_or("")),
                match &snap.profile {
                    AppliedProfile::Own { name, user, .. } => {
                        if *user {
                            i18n("Your profile %s is applied by falcond").replace("%s", name)
                        } else {
                            i18n("falcond's built-in profile %s is applied").replace("%s", name)
                        }
                    }
                    AppliedProfile::GenericProton => i18n(
                        "falcond's general Proton profile is applied: this game has no profile of its own",
                    ),
                    AppliedProfile::Other(name) => {
                        i18n("falcond applied the profile %s").replace("%s", name)
                    }
                    AppliedProfile::None => String::new(),
                },
            ),
            Headline::GameWithoutProfile => (
                "dialog-information-symbolic",
                i18n("%s is running without a profile")
                    .replace("%s", game_name.as_deref().unwrap_or("")),
                i18n(
                    "falcond has no profile for it, so nothing per game is applied. Create one in Profiles.",
                ),
            ),
            Headline::FalcondSilent => (
                "dialog-warning-symbolic",
                i18n("falcond is running but reports nothing"),
                i18n("Its status file could not be read. See Problems below."),
            ),
            Headline::FalcondFailed => (
                "dialog-error-symbolic",
                i18n("falcond failed"),
                i18n(
                    "The per-game optimization service failed. Logs show why; turning Turbo off and on again restarts it.",
                ),
            ),
        };
        let attention = snap.attention_count();
        if attention > 0 {
            subtitle.push_str(" · ");
            subtitle.push_str(&ni18n(
                "%n item needs attention",
                "%n items need attention",
                attention,
            ));
        }
        self.icon.set_icon_name(Some(icon));
        for c in ["overview-ok", "overview-warn", "overview-off"] {
            self.icon.remove_css_class(c);
        }
        self.icon.add_css_class(match snap.headline() {
            Headline::ReadyWaiting | Headline::Optimizing => "overview-ok",
            Headline::TurboOff => "overview-off",
            _ => "overview-warn",
        });
        self.title.set_label(&title);
        self.subtitle.set_label(&subtitle);
        self.subtitle.set_visible(!subtitle.is_empty());

        let game = snap.game.is_some();
        if let Some(c) = self.chip("turbo") {
            c.set(snap.turbo_state(), None);
        }
        if let Some(c) = self.chip("falcond") {
            c.set(snap.falcond_state(), None);
        }
        if let Some(c) = self.chip("profile") {
            match &snap.profile {
                AppliedProfile::Own { name, .. } | AppliedProfile::Other(name) => {
                    c.set(State::Active, Some(name));
                }
                AppliedProfile::GenericProton => {
                    c.set(State::Active, Some(&i18n("Proton (general)")));
                }
                AppliedProfile::None if game && snap.turbo_on => {
                    c.set(State::NotDetected, Some(&i18n("None")));
                }
                AppliedProfile::None => c.set(State::Off, Some(&i18n("None"))),
            }
        }
        if let Some(c) = self.chip("power") {
            c.set(snap.power_state(), snap.power_profile.as_deref());
        }
        if let Some(c) = self.chip("scheduler") {
            let text = snap.scheduler.loaded.clone().or_else(|| {
                (!snap.scheduler.requested.is_empty() && snap.scheduler.requested != "none")
                    .then(|| snap.scheduler.requested.clone())
            });
            c.set(snap.scheduler.state(game), text.as_deref());
        }
        if let Some(c) = self.chip("gpu") {
            let card = snap
                .game
                .as_ref()
                .and_then(|g| g.render_card.clone())
                .or_else(|| self.expected_gpu.clone());
            let name = card.as_ref().and_then(|c| self.gpu_names.get(c)).cloned();
            c.set(
                if game && snap.game.as_ref().is_some_and(|g| g.render_card.is_some()) {
                    State::Active
                } else {
                    State::Waiting
                },
                Some(name.as_deref().unwrap_or("—")),
            );
        }
        if let Some(c) = self.chip("gamescope") {
            c.set(snap.gamescope_state(), None);
        }
        if let Some(c) = self.chip("upscaling") {
            let (state, text) = upscaling_summary(snap);
            c.set(state, text.as_deref());
        }
        if let Some(c) = self.chip("framegen") {
            let (state, text) = frame_generation_summary(snap);
            c.set(state, text.as_deref());
        }
    }
}

/// One state for every upscaler: AI Graphics in the game, then Wine FSR,
/// then Gamescope's render size.
pub(crate) fn upscaling_summary(snap: &Snapshot) -> (State, Option<String>) {
    use bigame_core::graphics::runtime::Status;
    if let Some(Status::Active { upscaler, .. }) = &snap.ai_graphics {
        return (State::Active, Some(format!("OptiScaler {upscaler}")));
    }
    let gamescope_scales =
        snap.video.upscaling.base_width > 0 && snap.video.upscaling.gamescope_enabled;
    let wine = snap.wine_fsr_state();
    if wine != State::Off {
        return (wine, Some("Wine FSR".to_owned()));
    }
    if gamescope_scales {
        return (snap.gamescope_state(), Some("Gamescope".to_owned()));
    }
    (State::Off, None)
}

/// One state for frame generation: `OptiScaler`'s, then lsfg-vk.
pub(crate) fn frame_generation_summary(snap: &Snapshot) -> (State, Option<String>) {
    use bigame_core::graphics::runtime::Status;
    if snap.ai_frame_generation && matches!(snap.ai_graphics, Some(Status::Active { .. })) {
        return (State::Active, Some("OptiScaler".to_owned()));
    }
    let state = snap.frame_generation_state();
    let text = match (
        state,
        snap.in_game.as_ref().and_then(|g| g.frame_generation),
    ) {
        (State::Active, Some(m)) => Some(format!("lsfg-vk ×{m}")),
        (State::Off, _) => None,
        _ => Some("lsfg-vk".to_owned()),
    };
    (state, text)
}
