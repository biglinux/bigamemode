//! Real-time telemetry: six cards with a number and a sparkline.
//!
//! CPU frequency, RAM, disk activity and latency are read here, every second
//! while the page is on screen (latency every five, since it is a process).
//! The GPU load and temperature cards are filled by the GPU cards' own
//! reading (`gpus.rs`), which samples every card once.

use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;
use crate::widgets::sparkline::{self, SparkHandle};

/// Telemetry polling interval while the window has focus.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Behind another window: readings nobody can see cost a core's share for
/// nothing (measured at 4.4 % over two hours of games).
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);

/// The GPU cards' labels and sparklines, filled by the GPU reading.
#[derive(Clone)]
pub struct GpuTargets {
    pub load: gtk4::Label,
    pub temp: gtk4::Label,
    pub load_spark: SparkHandle,
    pub temp_spark: SparkHandle,
}

/// The telemetry group.
#[derive(Clone)]
pub struct Telemetry {
    group: adw::PreferencesGroup,
    cpu: (gtk4::Label, SparkHandle),
    ram: (gtk4::Label, SparkHandle),
    disk: (gtk4::Label, SparkHandle),
    ping: (gtk4::Label, SparkHandle),
    gpu: GpuTargets,
}

impl Telemetry {
    /// Build the group.
    #[must_use]
    pub fn new() -> Self {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Real-time Telemetry"));

        let grid = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(true)
            .column_spacing(12)
            .row_spacing(12)
            .min_children_per_line(1)
            .max_children_per_line(3)
            .build();

        let (cpu_card, cpu_val, cpu_spark) = card(&i18n("CPU Freq"), "cpu-symbolic");
        let (gpu_card, gpu_val, gpu_spark) = card(&i18n("GPU"), "video-display-symbolic");
        let (temp_card, temp_val, temp_spark) =
            card(&i18n("GPU Temp"), "freon-gpu-temperature-symbolic");
        let (ram_card, ram_val, ram_spark) = card(&i18n("RAM Usage"), "memory-symbolic");
        let (disk_card, disk_val, disk_spark) = card(&i18n("Disk I/O"), "drive-harddisk-symbolic");
        let (ping_card, ping_val, ping_spark) = card(&i18n("Latency"), "network-wireless-symbolic");
        for c in [
            cpu_card, gpu_card, temp_card, ram_card, disk_card, ping_card,
        ] {
            grid.insert(&c, -1);
        }
        group.add(&grid);

        Self {
            group,
            cpu: (cpu_val, cpu_spark),
            ram: (ram_val, ram_spark),
            disk: (disk_val, disk_spark),
            ping: (ping_val, ping_spark),
            gpu: GpuTargets {
                load: gpu_val,
                temp: temp_val,
                load_spark: gpu_spark,
                temp_spark,
            },
        }
    }

    /// The group.
    #[must_use]
    pub fn group(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    /// The GPU cards, for the GPU reading to fill.
    #[must_use]
    pub fn gpu_targets(&self) -> GpuTargets {
        self.gpu.clone()
    }

    /// Start reading, only while the group is on screen.
    pub fn start(&self) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let mut prev_disk: Option<(u64, u64)> = None;
            let mut last_ping: Option<std::time::Instant> = None;
            loop {
                if !this.group.is_mapped() {
                    glib::timeout_future(POLL_INTERVAL).await;
                    continue;
                }
                let cpu = gio::spawn_blocking(read_cpu_khz).await.ok().flatten();
                match cpu {
                    #[allow(clippy::cast_precision_loss)]
                    Some(khz) => {
                        this.cpu.1.push(khz as f64 / 1000.0);
                        this.cpu
                            .0
                            .set_text(&format!("{:.1} GHz", khz as f64 / 1_000_000.0));
                    }
                    None => this.cpu.0.set_text(&i18n("N/A")),
                }

                let cur_disk = gio::spawn_blocking(read_disk_sectors).await.ok().flatten();
                if let (Some(prev), Some(cur)) = (prev_disk, cur_disk) {
                    let read_kb = (cur.0.saturating_sub(prev.0) * 512) / 1024;
                    let write_kb = (cur.1.saturating_sub(prev.1) * 512) / 1024;
                    this.disk
                        .0
                        .set_text(&format!("{read_kb}R {write_kb}W KB/s"));
                    this.disk.1.push(f64::from(
                        u32::try_from(read_kb + write_kb).unwrap_or(u32::MAX),
                    ));
                } else {
                    this.disk.1.push(0.0);
                }
                prev_disk = cur_disk;

                if last_ping.is_none_or(|t| t.elapsed() >= BACKGROUND_INTERVAL) {
                    last_ping = Some(std::time::Instant::now());
                    let target = crate::settings::load().ping_target;
                    let ms = gio::spawn_blocking(move || read_ping_ms(&target))
                        .await
                        .ok()
                        .flatten();
                    match ms {
                        Some(ms) => {
                            this.ping.0.set_text(&format!("{ms} ms"));
                            if let Ok(v) = ms.parse::<f64>() {
                                this.ping.1.push(v);
                            }
                        }
                        None => this.ping.0.set_text(&i18n("Timeout")),
                    }
                }

                let ram = gio::spawn_blocking(read_ram_mb).await.ok().flatten();
                match ram {
                    #[allow(clippy::cast_precision_loss)]
                    Some((used, total)) if total > 0 => {
                        let perc = used as f64 / total as f64 * 100.0;
                        this.ram.1.push(perc);
                        this.ram
                            .0
                            .set_text(&format!("{:.1} GB ({perc:.0}%)", used as f64 / 1024.0));
                    }
                    _ => this.ram.0.set_text(&i18n("N/A")),
                }

                let wait = super::next_poll(&this.group, POLL_INTERVAL, BACKGROUND_INTERVAL);
                glib::timeout_future(wait).await;
            }
        });
    }
}

/// A telemetry card with an embedded sparkline.
fn card(title: &str, icon: &str) -> (gtk4::Box, gtk4::Label, SparkHandle) {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.add_css_class("telemetry-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let img = gtk4::Image::from_icon_name(icon);
    img.add_css_class("dim-label");
    header.append(&img);
    let title_lbl = gtk4::Label::new(Some(title));
    title_lbl.add_css_class("dim-label");
    title_lbl.add_css_class("caption");
    title_lbl.set_halign(gtk4::Align::Start);
    header.append(&title_lbl);
    card.append(&header);

    let val = gtk4::Label::new(Some("—"));
    val.add_css_class("title-2");
    val.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    val.set_lines(1);
    val.set_margin_top(4);
    val.set_halign(gtk4::Align::Start);
    card.append(&val);

    let spark = sparkline::build();
    spark.area.set_vexpand(true);
    spark.area.set_valign(gtk4::Align::End);
    spark.area.set_margin_top(6);
    card.append(&spark.area);

    (card, val, spark)
}

/// CPU core 0 frequency, kHz.
fn read_cpu_khz() -> Option<u64> {
    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Aggregate disk sectors (read, written) across whole block devices.
fn read_disk_sectors() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/diskstats").ok()?;
    let (mut read_total, mut write_total) = (0u64, 0u64);
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 10 {
            let dev = fields[2];
            // Whole devices only: no trailing digit for sd*, no pN for nvme.
            if dev.starts_with("sd") && dev.len() == 3
                || dev.starts_with("nvme") && dev.contains("n1") && !dev.contains('p')
                || dev.starts_with("vd") && dev.len() == 3
            {
                read_total += fields[5].parse::<u64>().unwrap_or(0);
                write_total += fields[9].parse::<u64>().unwrap_or(0);
            }
        }
    }
    Some((read_total, write_total))
}

/// One ICMP ping to `target`: the round trip, as ping prints it.
fn read_ping_ms(target: &str) -> Option<String> {
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

/// RAM in use and total, MB.
fn read_ram_mb() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |key: &str| {
        content
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
    };
    let total = field("MemTotal:")?;
    let avail = field("MemAvailable:")?;
    Some(((total.saturating_sub(avail)) / 1024, total / 1024))
}
