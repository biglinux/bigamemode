//! The machine's own readings for the Telemetry page: processor, memory,
//! disk and network.
//!
//! Each is one file the kernel keeps, cheap enough to read once a second.
//! Load, disk and network are counters: what they did is the difference
//! between two readings.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Processor time counters, from the first line of `/proc/stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    /// Time spent running something.
    pub busy: u64,
    /// All time, idle included.
    pub total: u64,
}

impl CpuTimes {
    /// Read the counters now.
    #[must_use]
    pub fn read() -> Option<Self> {
        parse_cpu_times(&std::fs::read_to_string("/proc/stat").ok()?)
    }

    /// How busy the processor was since `earlier`, percent of all its cores.
    #[must_use]
    pub fn load_since(self, earlier: Self) -> Option<f64> {
        let total = self.total.checked_sub(earlier.total)?;
        if total == 0 {
            return None;
        }
        let busy = self.busy.saturating_sub(earlier.busy);
        #[allow(clippy::cast_precision_loss)]
        Some((busy as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
    }
}

/// The aggregate `cpu` line of `/proc/stat`.
#[must_use]
pub fn parse_cpu_times(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal; guest time is already
    // counted in user, so the first eight are all of it.
    let total: u64 = v.iter().take(8).sum();
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some(CpuTimes {
        busy: total.saturating_sub(idle),
        total,
    })
}

/// The cores' clocks, kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuClocks {
    /// The fastest core right now: the one a game's main thread is on.
    pub fastest_khz: u64,
    /// The average over every core.
    pub average_khz: u64,
}

/// Every core's clock, from cpufreq.
#[must_use]
pub fn cpu_clocks() -> Option<CpuClocks> {
    let mut khz = Vec::new();
    for entry in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_cpu = name
            .strip_prefix("cpu")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !is_cpu {
            continue;
        }
        if let Some(f) = std::fs::read_to_string(entry.path().join("cpufreq/scaling_cur_freq"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            khz.push(f);
        }
    }
    clocks_of(&khz)
}

fn clocks_of(khz: &[u64]) -> Option<CpuClocks> {
    let fastest_khz = *khz.iter().max()?;
    let average_khz = khz.iter().sum::<u64>() / khz.len() as u64;
    Some(CpuClocks {
        fastest_khz,
        average_khz,
    })
}

/// The processor's temperature sensor drivers, AMD's then Intel's.
const CPU_SENSORS: [&str; 3] = ["k10temp", "zenpower", "coretemp"];
/// The sensor of the whole die or package, where the driver names one.
const CPU_SENSOR_LABELS: [&str; 3] = ["Tdie", "Tctl", "Package id 0"];

/// The processor's temperature, °C.
///
/// The sensor is looked for once; hwmon numbers do not change while the
/// machine is up.
#[must_use]
pub fn cpu_temp_c() -> Option<f64> {
    static SENSOR: OnceLock<Option<PathBuf>> = OnceLock::new();
    let input = SENSOR.get_or_init(|| cpu_sensor_in(Path::new("/sys/class/hwmon")));
    read_millidegrees(input.as_deref()?)
}

fn cpu_sensor_in(hwmon: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(hwmon)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for driver in CPU_SENSORS {
        let found = dirs
            .iter()
            .find(|d| std::fs::read_to_string(d.join("name")).is_ok_and(|n| n.trim() == driver));
        if let Some(dir) = found {
            return sensor_input(dir);
        }
    }
    None
}

fn sensor_input(dir: &Path) -> Option<PathBuf> {
    for want in CPU_SENSOR_LABELS {
        for i in 1..=32 {
            let label = std::fs::read_to_string(dir.join(format!("temp{i}_label")));
            if label.is_ok_and(|l| l.trim() == want) {
                return Some(dir.join(format!("temp{i}_input")));
            }
        }
    }
    let first = dir.join("temp1_input");
    first.exists().then_some(first)
}

fn read_millidegrees(input: &Path) -> Option<f64> {
    let milli: i64 = std::fs::read_to_string(input).ok()?.trim().parse().ok()?;
    #[allow(clippy::cast_precision_loss)]
    Some(milli as f64 / 1000.0)
}

/// Memory in use, MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    /// In use: all of it but what the kernel says is available.
    pub used_mib: u64,
    pub total_mib: u64,
    /// Swap in use.
    pub swap_used_mib: u64,
    pub swap_total_mib: u64,
}

impl Memory {
    /// Read `/proc/meminfo` now.
    #[must_use]
    pub fn read() -> Option<Self> {
        parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
}

/// `/proc/meminfo`.
#[must_use]
pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    let swap_total = field("SwapTotal:").unwrap_or(0);
    let swap_free = field("SwapFree:").unwrap_or(swap_total);
    Some(Memory {
        used_mib: total.saturating_sub(available) / 1024,
        total_mib: total / 1024,
        swap_used_mib: swap_total.saturating_sub(swap_free) / 1024,
        swap_total_mib: swap_total / 1024,
    })
}

/// Bytes read and written by every disk since boot.
///
/// Only the physical disks: a partition, a device-mapper or RAID volume, a
/// loop device or zram would count the same bytes twice, or count memory.
#[must_use]
pub fn disk_bytes() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/diskstats").ok()?;
    Some(parse_diskstats(&text, |dev| {
        Path::new("/sys/block").join(dev).join("device").exists()
    }))
}

/// `/proc/diskstats`, summed over the devices `physical` accepts.
pub fn parse_diskstats(text: &str, physical: impl Fn(&str) -> bool) -> (u64, u64) {
    let (mut read, mut written) = (0u64, 0u64);
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !physical(f[2]) {
            continue;
        }
        // Sectors are 512 bytes here, whatever the disk's own sector size.
        read += f[5].parse::<u64>().unwrap_or(0) * 512;
        written += f[9].parse::<u64>().unwrap_or(0) * 512;
    }
    (read, written)
}

/// Bytes received and sent through every network card since boot.
///
/// Only the cards: the loopback, bridges, tunnels and the virtual machines'
/// interfaces carry the same traffic again.
#[must_use]
pub fn network_bytes() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    Some(parse_net_dev(&text, |iface| {
        Path::new("/sys/class/net")
            .join(iface)
            .join("device")
            .exists()
    }))
}

/// `/proc/net/dev`, summed over the interfaces `physical` accepts.
pub fn parse_net_dev(text: &str, physical: impl Fn(&str) -> bool) -> (u64, u64) {
    let (mut received, mut sent) = (0u64, 0u64);
    for line in text.lines() {
        let Some((iface, counters)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface.contains('|') || !physical(iface) {
            continue;
        }
        let f: Vec<u64> = counters
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        if f.len() >= 9 {
            received += f[0];
            sent += f[8];
        }
    }
    (received, sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_load_is_busy_time_over_all_time() {
        let a = parse_cpu_times("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
        let b = parse_cpu_times("cpu  200 0 200 1300 300 0 0 0 0 0\n").unwrap();
        // 200 busy out of 1000.
        assert_eq!(b.load_since(a), Some(20.0));
        assert_eq!(a.load_since(a), None);
        assert_eq!(a.load_since(b), None);
        assert_eq!(parse_cpu_times("cpu0 1 2 3 4\n"), None);
    }

    #[test]
    fn clocks_name_the_fastest_core_and_the_average() {
        let c = clocks_of(&[4_000_000, 2_000_000, 3_000_000]).unwrap();
        assert_eq!((c.fastest_khz, c.average_khz), (4_000_000, 3_000_000));
        assert_eq!(clocks_of(&[]), None);
    }

    #[test]
    fn memory_in_use_is_what_is_not_available() {
        let m = parse_meminfo(
            "MemTotal:       32768000 kB\nMemFree:  1 kB\nMemAvailable:   16384000 kB\n\
             SwapTotal:       8192000 kB\nSwapFree:        7168000 kB\n",
        )
        .unwrap();
        assert_eq!((m.used_mib, m.total_mib), (16000, 32000));
        assert_eq!((m.swap_used_mib, m.swap_total_mib), (1000, 8000));
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn disks_count_whole_physical_devices_only() {
        let stats = "\
 259       0 nvme0n1 10 0 100 0 20 0 200 0 0 0 0
 259       1 nvme0n1p1 5 0 50 0 10 0 100 0 0 0 0
   8       0 sda 1 0 2 0 1 0 4 0 0 0 0
 254       0 dm-0 9 0 999 0 9 0 999 0 0 0 0
";
        let physical = |d: &str| d == "nvme0n1" || d == "sda";
        assert_eq!(parse_diskstats(stats, physical), (102 * 512, 204 * 512));
    }

    #[test]
    fn network_counts_the_cards_only() {
        let dev = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 5000 10 0 0 0 0 0 0 5000 10 0 0 0 0 0 0
enp5s0: 1000 10 0 0 0 0 0 0 300 5 0 0 0 0 0 0
virbr0: 700 1 0 0 0 0 0 0 700 1 0 0 0 0 0 0
 wlan0: 24 1 0 0 0 0 0 0 6 1 0 0 0 0 0 0
";
        let physical = |i: &str| i == "enp5s0" || i == "wlan0";
        assert_eq!(parse_net_dev(dev, physical), (1024, 306));
    }

    #[test]
    fn the_processor_sensor_is_the_die_one() {
        let dir = std::env::temp_dir().join(format!("bgm-hwmon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (n, name) in [(0, "amdgpu"), (1, "k10temp"), (2, "nvme")] {
            let h = dir.join(format!("hwmon{n}"));
            std::fs::create_dir_all(&h).unwrap();
            std::fs::write(h.join("name"), format!("{name}\n")).unwrap();
            std::fs::write(h.join("temp1_input"), "40000\n").unwrap();
        }
        let k10 = dir.join("hwmon1");
        std::fs::write(k10.join("temp1_label"), "Tctl\n").unwrap();
        std::fs::write(k10.join("temp3_label"), "Tccd1\n").unwrap();
        std::fs::write(k10.join("temp3_input"), "61500\n").unwrap();
        let input = cpu_sensor_in(&dir).unwrap();
        assert_eq!(input, k10.join("temp1_input"));
        std::fs::write(k10.join("temp1_input"), "61500\n").unwrap();
        assert_eq!(read_millidegrees(&input), Some(61.5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
