//! Telemetry: what the processor, memory, disk, network and every graphics
//! card are doing, second by second.
//!
//! The page is a fixed grid. Every card and every chart keeps its size and
//! its place whatever the readings say: a value that grows is cut short
//! rather than widening its card, a chart keeps its scale (a percentage
//! always tops at 100), and a metric the driver does not report says so in
//! its place instead of leaving a hole. Only the window's width changes the
//! grid: four cards in a row, two, or one ([`layout::Page`]).
//!
//! Readings are taken only while the page is in front of someone
//! ([`details::in_view`]): every second with the window focused, every five
//! behind another window, and not at all behind a game being played. All
//! of them are file reads off the main thread; the ping, a process, runs on
//! its own every five seconds.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::gpu_telemetry::{ClockLimit, GpuSample};
use bigame_core::graphics::report::GpuInfo;
use bigame_core::hardware::{Gpu, Hardware};
use bigame_core::system_telemetry::{self as sys, CpuTimes, Memory};

use crate::i18n::i18n;
use crate::views::details;
use crate::widgets::layout::{self, fixed_label, row_box};
use crate::widgets::sparkline::{self, Scale, SparkHandle};

/// Reading interval while the window has focus.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Behind another window: readings nobody can see cost a core's share for
/// nothing (measured at 4.4 % over two hours of games).
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);
/// The latency is a process (`ping`), so it is measured less often.
const PING_INTERVAL: Duration = Duration::from_secs(5);

/// The height of the machine's charts and of the graphics cards' ones.
const CHART_HEIGHT: i32 = 56;
const GPU_CHART_HEIGHT: i32 = 44;

/// Build the Telemetry page.
///
/// The machine's cards are there at once; the graphics cards are added the
/// first time the page is on screen, once the hardware has been read off the
/// main thread.
#[must_use]
// The page's layout in one place, top to bottom.
#[allow(clippy::too_many_lines)]
pub fn build() -> gtk4::Widget {
    let page = layout::Page::new();
    page.intro(&i18n(
        "Live readings, once a second. They pause while a game has the screen, so watching never costs the game anything.",
    ));

    page.section(&i18n("This computer"));
    let cpu = Panel::new(
        &i18n("Processor"),
        "bigame-cpu-symbolic",
        Scale::Fixed(100.0),
    );
    let memory = Panel::new(
        &i18n("Memory"),
        "bigame-memory-symbolic",
        Scale::Fixed(100.0),
    );
    // Disk and network in KB/s and ms: the floor keeps a quiet second flat.
    let disk = Panel::new(
        &i18n("Disk"),
        "drive-harddisk-symbolic",
        Scale::AtLeast(1024.0),
    );
    let network = Panel::new(
        &i18n("Network"),
        "network-transmit-receive-symbolic",
        Scale::AtLeast(50.0),
    );
    page.row(&[
        cpu.root.upcast_ref(),
        memory.root.upcast_ref(),
        disk.root.upcast_ref(),
        network.root.upcast_ref(),
    ]);

    page.section(&i18n("Graphics cards"));
    let gpu_row = layout::row_box();
    page.stack_when_narrow(&gpu_row);
    page.add(&gpu_row);

    let machine_panels = Machine {
        cpu,
        memory,
        disk,
        network,
    };
    let root = page.widget().clone();
    let widget = root.clone().upcast::<gtk4::Widget>();
    glib::spawn_future_local(async move {
        details::mapped(&root).await;
        let found = gio::spawn_blocking(|| {
            let hw = Hardware::detect();
            let (infos, _) = bigame_core::graphics::report::gpu_infos(&hw, None);
            (hw, infos)
        })
        .await;
        let Ok((hw, infos)) = found else { return };
        let cards: Vec<GpuCard> = infos
            .iter()
            .map(|info| {
                let card = GpuCard::new(info);
                gpu_row.append(&card.root);
                page.stack_when_tiny(&card.charts);
                card
            })
            .collect();
        if cards.is_empty() {
            gpu_row.append(&no_gpu());
        }
        read_forever(&root, &machine_panels, &cards, Arc::new(hw.gpus)).await;
    });

    widget
}

/// One of the machine's four cards: a value, two lines under it, a chart.
#[derive(Clone)]
struct Panel {
    root: gtk4::Box,
    value: gtk4::Label,
    first: gtk4::Label,
    second: gtk4::Label,
    chart: SparkHandle,
}

impl Panel {
    fn new(title: &str, icon: &str, scale: Scale) -> Self {
        let root = layout::card(title, icon);

        let value = fixed_label("—", &["title-2", "telemetry-value"]);
        value.set_margin_top(6);
        root.append(&value);
        let first = fixed_label("—", &["caption", "telemetry-detail"]);
        let second = fixed_label("—", &["caption", "dim-label", "telemetry-detail"]);
        root.append(&first);
        root.append(&second);

        let chart = sparkline::chart(scale, CHART_HEIGHT);
        chart.area.set_margin_top(8);
        root.append(&chart.area);

        Self {
            root,
            value,
            first,
            second,
            chart,
        }
    }
}

/// The machine's four cards.
struct Machine {
    cpu: Panel,
    memory: Panel,
    disk: Panel,
    network: Panel,
}

/// A graphics card's figure: a name, a value, and a chart or a bar.
#[derive(Clone)]
struct Figure {
    root: gtk4::Box,
    name: gtk4::Label,
    value: gtk4::Label,
    chart: Option<SparkHandle>,
    bar: Option<gtk4::LevelBar>,
}

impl Figure {
    fn chart(name: &str) -> Self {
        let f = Self::plain(name, "title-3");
        let chart = sparkline::chart(Scale::Fixed(100.0), GPU_CHART_HEIGHT);
        chart.area.set_margin_top(6);
        f.root.append(&chart.area);
        Self {
            chart: Some(chart),
            ..f
        }
    }

    fn bar(name: &str) -> Self {
        let f = Self::plain(name, "heading");
        let bar = gtk4::LevelBar::builder()
            .min_value(0.0)
            .max_value(100.0)
            .value(0.0)
            .margin_top(6)
            .css_classes(["gpu-bar"])
            .build();
        // One colour, whatever the level: the number says how much.
        bar.remove_offset_value(Some(gtk4::LEVEL_BAR_OFFSET_LOW));
        bar.remove_offset_value(Some(gtk4::LEVEL_BAR_OFFSET_HIGH));
        bar.remove_offset_value(Some(gtk4::LEVEL_BAR_OFFSET_FULL));
        f.root.append(&bar);
        Self {
            bar: Some(bar),
            ..f
        }
    }

    fn plain(name: &str, value_class: &str) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        root.add_css_class("telemetry-tile");
        root.set_hexpand(true);
        let name = fixed_label(name, &["caption", "dim-label"]);
        let value = fixed_label("—", &[value_class, "telemetry-value"]);
        root.append(&name);
        root.append(&value);
        Self {
            root,
            name,
            value,
            chart: None,
            bar: None,
        }
    }

    /// Show `text`, or that the driver does not report it.
    fn set(&self, text: Option<String>) {
        if let Some(t) = text {
            self.value.set_label(&t);
            self.value.remove_css_class("dim-label");
        } else {
            self.value.set_label(&i18n("Not reported"));
            self.value.add_css_class("dim-label");
        }
    }
}

/// One graphics card's card.
struct GpuCard {
    card: String,
    root: gtk4::Box,
    role: gtk4::Label,
    /// The two charts, side by side; a breakpoint stacks them.
    charts: gtk4::Box,
    load: Figure,
    temp: Figure,
    clock: Figure,
    vram: Figure,
    power: Figure,
}

impl GpuCard {
    fn new(info: &GpuInfo) -> Self {
        let name = info
            .product
            .clone()
            .unwrap_or_else(|| bigame_core::graphics::report::display_name(&info.name));
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        root.add_css_class("card");
        root.add_css_class("telemetry-card");
        root.set_hexpand(true);
        root.update_property(&[gtk4::accessible::Property::Label(&name)]);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        let icon = gtk4::Image::from_icon_name("video-display-symbolic");
        icon.set_pixel_size(24);
        icon.add_css_class("dim-label");
        header.append(&icon);
        let names = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        names.set_hexpand(true);
        names.append(&fixed_label(&name, &["heading"]));
        let kind = if info.discrete {
            i18n("Dedicated")
        } else {
            i18n("Integrated")
        };
        let driver = match &info.userspace {
            Some(u) => format!("{kind} · {} · {u}", info.driver),
            None => format!("{kind} · {}", info.driver),
        };
        names.append(&fixed_label(&driver, &["caption", "dim-label"]));
        header.append(&names);
        let role = layout::pill(&i18n("Checking…"));
        header.append(&role);
        root.append(&header);

        let load = Figure::chart(&i18n("Load"));
        let temp = Figure::chart(&i18n("Temperature"));
        let charts = row_box();
        charts.append(&load.root);
        charts.append(&temp.root);
        root.append(&charts);

        let clock = Figure::plain(&i18n("Clock"), "heading");
        let vram = Figure::bar(&i18n("Video memory"));
        let power = Figure::plain(&i18n("Power"), "heading");
        let stats = row_box();
        for f in [&clock, &vram, &power] {
            stats.append(&f.root);
        }
        root.append(&stats);

        Self {
            card: info.card.clone(),
            root,
            role,
            charts,
            load,
            temp,
            clock,
            vram,
            power,
        }
    }

    fn show(&self, s: Option<&GpuSample>, games_card: bool, game: Option<&str>) {
        let asleep = s.is_some_and(|s| s.asleep);
        let role = if asleep {
            i18n("Asleep")
        } else {
            match (game, games_card) {
                (Some(name), true) => i18n("Renders %s").replace("%s", name),
                (None, true) => i18n("Games render here"),
                _ => i18n("Available"),
            }
        };
        self.role.set_label(&role);
        self.role.set_tooltip_text(Some(&role));
        set_class(
            &self.role,
            &["telemetry-role-active", "telemetry-role-idle"],
            if games_card && !asleep {
                "telemetry-role-active"
            } else {
                "telemetry-role-idle"
            },
        );

        let Some(s) = s.filter(|s| !s.asleep) else {
            // Asleep, or not read: the same places, saying so.
            let text = if asleep {
                i18n("Asleep")
            } else {
                "—".to_owned()
            };
            for f in [&self.load, &self.temp, &self.clock, &self.vram, &self.power] {
                f.value.set_label(&text);
                f.value.add_css_class("dim-label");
            }
            self.temp.name.set_label(&i18n("Temperature"));
            if let Some(bar) = &self.vram.bar {
                bar.set_value(0.0);
            }
            return;
        };

        self.load.set(s.busy_pct.map(|b| format!("{b}%")));
        if let (Some(chart), Some(b)) = (&self.load.chart, s.busy_pct) {
            chart.push(f64::from(b));
        }

        self.temp.set(s.temp_c.map(|t| format!("{t:.0}°C")));
        let class = temp_class(s.temp_c);
        set_class(
            &self.temp.value,
            &["temp-normal", "temp-warm", "temp-hot"],
            class,
        );
        if let Some(chart) = &self.temp.chart {
            chart.set_color(chart_color(class));
            if let Some(t) = s.temp_c {
                chart.push(t);
            }
        }
        // What holds the clock down, where the driver says (NVIDIA).
        let mut name = i18n("Temperature");
        for l in &s.limited_by {
            name.push_str(" · ");
            name.push_str(&match l {
                ClockLimit::PowerCap => i18n("power limit"),
                ClockLimit::Thermal => i18n("temperature limit"),
                ClockLimit::HardwareSlowdown => i18n("hardware slowdown"),
            });
        }
        self.temp.name.set_tooltip_text(Some(&name));
        self.temp.name.set_label(&name);

        self.clock.set(s.clock_mhz.map(|c| {
            if c >= 1000 {
                format!("{:.2} GHz", f64::from(c) / 1000.0)
            } else {
                format!("{c} MHz")
            }
        }));
        #[allow(clippy::cast_precision_loss)]
        let vram = match (s.vram_used_mib, s.vram_total_mib) {
            (Some(u), Some(t)) if t > 0 => Some((
                format!("{:.1} / {}", u as f64 / 1024.0, gb_total(t)),
                u as f64 / t as f64 * 100.0,
            )),
            (Some(u), _) => Some((format!("{:.1} GB", u as f64 / 1024.0), 0.0)),
            _ => None,
        };
        if let Some(bar) = &self.vram.bar {
            bar.set_value(vram.as_ref().map_or(0.0, |v| v.1.clamp(0.0, 100.0)));
        }
        self.vram.set(vram.map(|v| v.0));
        self.power.set(s.power_w.map(|w| format!("{w:.0} W")));
    }
}

/// Shown in place of the graphics cards when the kernel exposes none.
fn no_gpu() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    b.add_css_class("card");
    b.add_css_class("telemetry-card");
    b.append(&fixed_label(&i18n("No GPU was found"), &["heading"]));
    b.append(&fixed_label(
        &i18n("No DRM card is exposed by the kernel."),
        &["caption", "dim-label"],
    ));
    b
}

/// Replace whichever of `all` the widget has with `one`.
fn set_class(widget: &impl IsA<gtk4::Widget>, all: &[&str], one: &str) {
    for c in all {
        if *c != one {
            widget.remove_css_class(c);
        }
    }
    widget.add_css_class(one);
}

/// The colour class of a temperature, as the rest of the application uses.
fn temp_class(temp: Option<f64>) -> &'static str {
    match temp {
        Some(t) if t.round() > 80.0 => "temp-hot",
        Some(t) if t.round() > 60.0 => "temp-warm",
        _ => "temp-normal",
    }
}

/// The line colour of a temperature chart: the theme's own until warm.
fn chart_color(class: &str) -> Option<(f64, f64, f64)> {
    match class {
        "temp-warm" => Some((1.0, 0.65, 0.0)),
        "temp-hot" => Some((0.9, 0.15, 0.15)),
        _ => None,
    }
}

/// A rate in bytes per second, e.g. `12.3 MB/s`.
#[allow(clippy::cast_precision_loss)]
fn rate_text(bytes_per_s: f64) -> String {
    let kb = bytes_per_s / 1024.0;
    if kb >= 1024.0 {
        format!("{:.1} MB/s", kb / 1024.0)
    } else {
        format!("{kb:.0} KB/s")
    }
}

/// Gigabytes from MiB, e.g. `9.4 GB`.
#[allow(clippy::cast_precision_loss)]
fn gb(mib: u64) -> String {
    format!("{:.1} GB", mib as f64 / 1024.0)
}

/// A card's whole video memory: `16 GB`, or `0.5 GB` for the share an
/// integrated GPU is given.
#[allow(clippy::cast_precision_loss)]
fn gb_total(mib: u64) -> String {
    let gb = mib as f64 / 1024.0;
    if gb < 10.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{gb:.0} GB")
    }
}

/// One reading of everything but the ping.
#[derive(Default)]
struct Reading {
    cpu_times: Option<CpuTimes>,
    clocks: Option<sys::CpuClocks>,
    cpu_temp: Option<f64>,
    memory: Option<Memory>,
    disk: Option<(u64, u64)>,
    network: Option<(u64, u64)>,
    /// The games' card, by index, and every card's sample.
    games_gpu: Option<usize>,
    gpus: Vec<GpuSample>,
}

impl Reading {
    fn take(gpus: &[Gpu], render_card: Option<&str>) -> Self {
        Self {
            cpu_times: CpuTimes::read(),
            clocks: sys::cpu_clocks(),
            cpu_temp: sys::cpu_temp_c(),
            memory: Memory::read(),
            disk: sys::disk_bytes(),
            network: sys::network_bytes(),
            games_gpu: bigame_core::gpu_telemetry::games_gpu(gpus, render_card),
            gpus: gpus
                .iter()
                .map(bigame_core::gpu_telemetry::sample)
                .collect(),
        }
    }
}

/// Read and show, for as long as the window lives.
async fn read_forever(
    root: &adw::BreakpointBin,
    m: &Machine,
    cards: &[GpuCard],
    gpus: Arc<Vec<Gpu>>,
) {
    let mut previous: Option<(Instant, Reading)> = None;
    let mut last_ping: Option<Instant> = None;
    let pinging = Rc::new(Cell::new(false));
    loop {
        details::in_view(root).await;
        // A reading older than a pause is no base for a rate.
        if previous
            .as_ref()
            .is_some_and(|(t, _)| t.elapsed() > BACKGROUND_INTERVAL * 2)
        {
            previous = None;
        }

        let game = crate::game_watch::current();
        let render_card = game.as_ref().and_then(|g| g.render_card.clone());
        let game_name = game.map(|g| g.display_name);
        let list = Arc::clone(&gpus);
        let now = gio::spawn_blocking(move || Reading::take(&list, render_card.as_deref()))
            .await
            .unwrap_or_default();
        let at = Instant::now();
        let prev = previous
            .as_ref()
            .map(|(t, r)| (at.duration_since(*t).as_secs_f64(), r));

        show_cpu(&m.cpu, &now, prev.map(|p| p.1));
        show_memory(&m.memory, now.memory);
        show_disk(&m.disk, &now, prev);
        show_network(&m.network, &now, prev);
        for (i, card) in cards.iter().enumerate() {
            let s = now.gpus.iter().find(|s| s.card == card.card);
            card.show(s, now.games_gpu == Some(i), game_name.as_deref());
        }

        if last_ping.is_none_or(|t| t.elapsed() >= PING_INTERVAL) && !pinging.replace(true) {
            last_ping = Some(Instant::now());
            let (panel, pinging) = (m.network.clone(), Rc::clone(&pinging));
            glib::spawn_future_local(async move {
                let target = crate::settings::load().ping_target;
                let shown = target.clone();
                let ms = gio::spawn_blocking(move || read_ping_ms(&target))
                    .await
                    .ok()
                    .flatten();
                pinging.set(false);
                panel
                    .second
                    .set_label(&i18n("Ping to %s").replace("%s", &shown));
                if let Some(ms) = ms.and_then(|m| m.parse::<f64>().ok()) {
                    panel.value.set_label(&format!("{ms:.0} ms"));
                    panel.value.remove_css_class("dim-label");
                    panel.chart.push(ms);
                } else {
                    panel.value.set_label(&i18n("Timeout"));
                    panel.value.add_css_class("dim-label");
                }
            });
        }

        previous = Some((at, now));
        let wait = details::next_poll(root, POLL_INTERVAL, BACKGROUND_INTERVAL);
        glib::timeout_future(wait).await;
    }
}

fn show_cpu(p: &Panel, now: &Reading, prev: Option<&Reading>) {
    let load = now
        .cpu_times
        .zip(prev.and_then(|r| r.cpu_times))
        .and_then(|(n, p)| n.load_since(p));
    if let Some(l) = load {
        p.value.set_label(&format!("{l:.0}%"));
        p.chart.push(l);
    }
    #[allow(clippy::cast_precision_loss)]
    let clocks = now.clocks.map(|c| {
        i18n("Fastest core %f · average %a")
            .replace("%f", &format!("{:.2} GHz", c.fastest_khz as f64 / 1e6))
            .replace("%a", &format!("{:.1} GHz", c.average_khz as f64 / 1e6))
    });
    p.first
        .set_label(&clocks.unwrap_or_else(|| i18n("Clock not reported")));
    match now.cpu_temp {
        Some(t) => {
            p.second
                .set_label(&i18n("Temperature %s").replace("%s", &format!("{t:.0}°C")));
            set_class(
                &p.second,
                &["dim-label", "temp-normal", "temp-warm", "temp-hot"],
                temp_class(Some(t)),
            );
        }
        None => p.second.set_label(&i18n("Temperature not reported")),
    }
}

fn show_memory(p: &Panel, memory: Option<Memory>) {
    let Some(m) = memory.filter(|m| m.total_mib > 0) else {
        p.value.set_label(&i18n("N/A"));
        return;
    };
    #[allow(clippy::cast_precision_loss)]
    let pct = m.used_mib as f64 / m.total_mib as f64 * 100.0;
    p.value.set_label(&gb(m.used_mib));
    p.first.set_label(
        &i18n("of %t in use (%p)")
            .replace("%t", &gb(m.total_mib))
            .replace("%p", &format!("{pct:.0}%")),
    );
    p.second.set_label(&if m.swap_total_mib == 0 {
        i18n("No swap")
    } else {
        i18n("Swap %u of %t")
            .replace("%u", &gb(m.swap_used_mib))
            .replace("%t", &gb(m.swap_total_mib))
    });
    p.chart.push(pct);
}

/// The rates of two counter pairs `secs` apart, per second.
#[allow(clippy::cast_precision_loss)]
fn rates(now: Option<(u64, u64)>, prev: Option<(u64, u64)>, secs: f64) -> Option<(f64, f64)> {
    let (n, p) = (now?, prev?);
    if secs <= 0.0 {
        return None;
    }
    Some((
        n.0.saturating_sub(p.0) as f64 / secs,
        n.1.saturating_sub(p.1) as f64 / secs,
    ))
}

fn show_disk(p: &Panel, now: &Reading, prev: Option<(f64, &Reading)>) {
    let Some((read, written)) = prev.and_then(|(secs, r)| rates(now.disk, r.disk, secs)) else {
        return;
    };
    p.value.set_label(&rate_text(read + written));
    p.first
        .set_label(&i18n("Read %s").replace("%s", &rate_text(read)));
    p.second
        .set_label(&i18n("Written %s").replace("%s", &rate_text(written)));
    p.chart.push((read + written) / 1024.0);
}

fn show_network(p: &Panel, now: &Reading, prev: Option<(f64, &Reading)>) {
    if let Some((down, up)) = prev.and_then(|(secs, r)| rates(now.network, r.network, secs)) {
        p.first
            .set_label(&format!("↓ {}  ↑ {}", rate_text(down), rate_text(up)));
    }
}

/// CPU core 0 frequency, kHz.
pub(crate) fn read_cpu_khz() -> Option<u64> {
    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// One ICMP ping to `target`: the round trip, as ping prints it.
pub(crate) fn read_ping_ms(target: &str) -> Option<String> {
    // Passed as an argument, never through a shell; a target beginning with
    // '-' would still be read as an option by ping.
    if target.is_empty() || target.starts_with('-') {
        return None;
    }
    let out = std::process::Command::new("ping")
        .args(["-c", "1", "-W", "1", target])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // "rtt min/avg/max/mdev = X/Y/Z/W ms" is not translated; "time=" is.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rest = stdout.lines().find_map(|l| l.strip_prefix("rtt "))?;
    let values = rest[rest.find('=')? + 1..].trim();
    Some(values[..values.find('/')?].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_are_per_second() {
        assert_eq!(
            rates(Some((3000, 600)), Some((1000, 100)), 2.0),
            Some((1000.0, 250.0))
        );
        assert_eq!(rates(Some((1, 1)), None, 1.0), None);
        assert_eq!(rates(Some((1, 1)), Some((0, 0)), 0.0), None);
        // A counter that went back (a disk removed) is not a negative rate.
        assert_eq!(rates(Some((0, 0)), Some((10, 10)), 1.0), Some((0.0, 0.0)));
    }

    #[test]
    fn rate_text_switches_to_megabytes() {
        assert_eq!(rate_text(0.0), "0 KB/s");
        assert_eq!(rate_text(512.0 * 1024.0), "512 KB/s");
        assert_eq!(rate_text(12.3 * 1024.0 * 1024.0), "12.3 MB/s");
    }

    #[test]
    fn small_video_memory_keeps_its_decimal() {
        assert_eq!(gb_total(512), "0.5 GB");
        assert_eq!(gb_total(16 * 1024), "16 GB");
    }

    #[test]
    fn temperatures_take_the_application_colours() {
        assert_eq!(temp_class(None), "temp-normal");
        assert_eq!(temp_class(Some(60.0)), "temp-normal");
        assert_eq!(temp_class(Some(61.0)), "temp-warm");
        assert_eq!(temp_class(Some(81.0)), "temp-hot");
    }
}
