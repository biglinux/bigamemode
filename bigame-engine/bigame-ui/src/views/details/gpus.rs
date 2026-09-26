//! One card per GPU: which one renders the game, and what it is doing.
//!
//! Only what the driver reports is shown; a metric it does not publish is
//! left out rather than shown as zero. On a hybrid machine the card that
//! renders the running game is named as such, the others as available or
//! asleep.

use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::gpu_telemetry::GpuSample;
use bigame_core::hardware::Hardware;

use crate::i18n::i18n;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);

/// One GPU's card.
#[derive(Clone)]
struct GpuCard {
    card: String,
    role: gtk4::Label,
    load: Metric,
    clock: Metric,
    vram: Metric,
    temp: Metric,
    power: Metric,
}

/// A metric: a name, a value, and a bar when the metric has a range.
#[derive(Clone)]
struct Metric {
    row: gtk4::Box,
    value: gtk4::Label,
    bar: Option<gtk4::LevelBar>,
}

impl Metric {
    fn new(name: &str, with_bar: bool) -> Self {
        let label = gtk4::Label::builder()
            .label(name)
            .css_classes(["caption", "dim-label"])
            .xalign(0.0)
            .width_chars(11)
            .build();
        let value = gtk4::Label::builder()
            .label("—")
            .xalign(1.0)
            .css_classes(["caption", "gpu-metric-value"])
            .hexpand(true)
            .build();
        let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        line.append(&label);
        line.append(&value);
        row.append(&line);
        let bar = with_bar.then(|| {
            let bar = gtk4::LevelBar::builder()
                .min_value(0.0)
                .max_value(100.0)
                .value(0.0)
                .css_classes(["gpu-bar"])
                .build();
            // One colour, whatever the level: the number says how much.
            bar.remove_offset_value(Some(gtk4::LEVEL_BAR_OFFSET_LOW));
            bar.remove_offset_value(Some(gtk4::LEVEL_BAR_OFFSET_HIGH));
            bar.remove_offset_value(Some(gtk4::LEVEL_BAR_OFFSET_FULL));
            row.append(&bar);
            bar
        });
        row.set_visible(false);
        Self { row, value, bar }
    }

    fn show(&self, text: Option<String>, fraction: Option<f64>) {
        match text {
            Some(t) => {
                self.value.set_label(&t);
                self.row.set_visible(true);
            }
            None => self.row.set_visible(false),
        }
        if let (Some(bar), Some(f)) = (&self.bar, fraction) {
            bar.set_value((f * 100.0).clamp(0.0, 100.0));
        }
    }
}

/// The GPU cards group.
#[derive(Clone)]
pub struct Gpus {
    group: adw::PreferencesGroup,
    cards: Vec<GpuCard>,
}

impl Gpus {
    /// Build one card per GPU found.
    #[must_use]
    pub fn new(hw: &Hardware) -> Self {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Graphics cards"));
        group.set_description(Some(&i18n(
            "Which card renders the running game, and what each one is doing. Only what the driver reports is shown.",
        )));
        let (infos, _) = bigame_core::graphics::report::gpu_infos(hw, None);
        let grid = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(true)
            .column_spacing(12)
            .row_spacing(12)
            .min_children_per_line(1)
            .max_children_per_line(2)
            .build();
        let mut cards = Vec::new();
        for g in infos {
            let title = gtk4::Label::builder()
                .label(bigame_core::graphics::report::display_name(&g.name))
                .css_classes(["heading"])
                .xalign(0.0)
                .wrap(true)
                .build();
            let role = gtk4::Label::builder()
                .label(i18n("Checking…"))
                .css_classes(["caption", "gpu-role"])
                .xalign(0.0)
                .wrap(true)
                .build();
            let driver = gtk4::Label::builder()
                .label(match &g.userspace {
                    Some(u) => format!("{} · {u}", g.driver),
                    None => g.driver.clone(),
                })
                .css_classes(["caption", "dim-label"])
                .xalign(0.0)
                .wrap(true)
                .build();

            let load = Metric::new(&i18n("Load"), true);
            let clock = Metric::new(&i18n("Clock"), false);
            let vram = Metric::new(&i18n("VRAM"), true);
            let temp = Metric::new(&i18n("Temperature"), false);
            let power = Metric::new(&i18n("Power"), false);

            let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            card.add_css_class("card");
            card.add_css_class("gpu-card");
            card.set_hexpand(true);
            card.append(&title);
            card.append(&role);
            card.append(&driver);
            for m in [&load, &clock, &vram, &temp, &power] {
                card.append(&m.row);
            }
            grid.insert(&card, -1);
            cards.push(GpuCard {
                card: g.card,
                role,
                load,
                clock,
                vram,
                temp,
                power,
            });
        }
        if cards.is_empty() {
            let row = adw::ActionRow::builder()
                .title(i18n("No GPU was found"))
                .subtitle(i18n("No DRM card is exposed by the kernel."))
                .build();
            group.add(&row);
        } else {
            group.add(&grid);
        }
        Self { group, cards }
    }

    /// The group.
    #[must_use]
    pub fn group(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    /// Sample every card while the page is on screen; the games' card also
    /// fills the telemetry cards.
    pub fn start(&self, hw: &std::rc::Rc<Hardware>, targets: super::telemetry::GpuTargets) {
        let this = self.clone();
        let hw = std::rc::Rc::clone(hw);
        glib::spawn_future_local(async move {
            loop {
                if !this.group.is_mapped() {
                    glib::timeout_future(POLL_INTERVAL).await;
                    continue;
                }
                let game = crate::game_watch::current();
                let render_card = game.as_ref().and_then(|g| g.render_card.clone());
                let game_name = game.map(|g| g.display_name);
                let list = hw.gpus.clone();
                let read = gio::spawn_blocking(move || {
                    let at = bigame_core::gpu_telemetry::games_gpu(&list, render_card.as_deref());
                    let samples: Vec<_> = list
                        .iter()
                        .map(bigame_core::gpu_telemetry::sample)
                        .collect();
                    (at, samples)
                })
                .await;
                if let Ok((at, samples)) = read {
                    if let Some(s) = at.and_then(|i| samples.get(i)) {
                        fill_targets(&targets, s);
                    }
                    for (i, card) in this.cards.iter().enumerate() {
                        let s = samples.iter().find(|s| s.card == card.card);
                        let games_card = at == Some(i);
                        card.show(s, games_card, game_name.as_deref());
                    }
                }
                let wait = super::next_poll(&this.group, POLL_INTERVAL, BACKGROUND_INTERVAL);
                glib::timeout_future(wait).await;
            }
        });
    }
}

impl GpuCard {
    fn show(&self, s: Option<&GpuSample>, games_card: bool, game: Option<&str>) {
        let mut role = match (game, games_card) {
            (Some(name), true) => i18n("Renders %s").replace("%s", name),
            (None, true) => i18n("Games render here"),
            _ => i18n("Available"),
        };
        if s.is_some_and(|s| s.asleep) {
            role.push_str(" · ");
            role.push_str(&i18n("Asleep"));
        }
        self.role.set_label(&role);
        for c in ["gpu-role-active", "gpu-role-idle"] {
            self.role.remove_css_class(c);
        }
        self.role.add_css_class(if games_card {
            "gpu-role-active"
        } else {
            "gpu-role-idle"
        });
        let Some(s) = s.filter(|s| !s.asleep) else {
            for m in [&self.load, &self.clock, &self.vram, &self.temp, &self.power] {
                m.show(None, None);
            }
            return;
        };
        self.load.show(
            s.busy_pct.map(|b| format!("{b}%")),
            s.busy_pct.map(|b| f64::from(b) / 100.0),
        );
        self.clock.show(
            s.clock_mhz.map(|c| {
                if c >= 1000 {
                    format!("{:.2} GHz", f64::from(c) / 1000.0)
                } else {
                    format!("{c} MHz")
                }
            }),
            None,
        );
        #[allow(clippy::cast_precision_loss)]
        self.vram.show(
            match (s.vram_used_mib, s.vram_total_mib) {
                (Some(u), Some(t)) if t > 0 => Some(format!(
                    "{:.1} / {:.0} GB",
                    u as f64 / 1024.0,
                    t as f64 / 1024.0
                )),
                (Some(u), _) => Some(format!("{:.1} GB", u as f64 / 1024.0)),
                _ => None,
            },
            match (s.vram_used_mib, s.vram_total_mib) {
                (Some(u), Some(t)) if t > 0 => Some(u as f64 / t as f64),
                _ => None,
            },
        );
        let (temp_text, class) = crate::gpu_reading::temp_text(s);
        for c in ["temp-normal", "temp-warm", "temp-hot"] {
            self.temp.value.remove_css_class(c);
        }
        self.temp.value.add_css_class(class);
        self.temp.show(s.temp_c.map(|_| temp_text), None);
        self.power
            .show(s.power_w.map(|w| format!("{w:.0} W")), None);
    }
}

/// The telemetry cards for the games' GPU.
fn fill_targets(t: &super::telemetry::GpuTargets, s: &GpuSample) {
    t.load.set_text(&crate::gpu_reading::load_text(s));
    if let Some(v) = crate::gpu_reading::spark_value(s) {
        t.load_spark.push(v);
    }
    let (text, class) = crate::gpu_reading::temp_text(s);
    for c in ["temp-normal", "temp-warm", "temp-hot"] {
        t.temp.remove_css_class(c);
    }
    t.temp.add_css_class(class);
    t.temp.set_text(&text);
    t.temp_spark.set_color(match class {
        "temp-warm" => Some((1.0, 0.65, 0.0)),
        "temp-hot" => Some((0.9, 0.15, 0.15)),
        _ => None,
    });
    if let Some(temp) = s.temp_c {
        t.temp_spark.push(temp);
    }
}
