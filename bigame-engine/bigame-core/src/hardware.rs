//! Hardware discovery.
//!
//! Everything here is *observation only* — no file in this module ever writes to
//! the system. The result is the ground truth the Booster planner reasons over,
//! so a wrong answer here silently becomes a wrong optimization later. Each
//! field therefore records what was actually read, and uses `Option` rather than
//! a guessed default whenever the system did not tell us.

use std::path::{Path, PathBuf};

// ── CPU ──────────────────────────────────────────────────────────────────────

/// CPU manufacturer, as reported by `/proc/cpuinfo`'s `vendor_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    /// `AuthenticAMD`.
    Amd,
    /// `GenuineIntel`.
    Intel,
    /// Anything else (including virtualised CPUs with odd vendor strings).
    Other,
}

/// AMD 3D V-Cache control device, when the platform driver bound one.
#[derive(Debug, Clone)]
pub struct VCacheDevice {
    /// The `amd_x3d_mode` attribute to read and write.
    ///
    /// Discovered by globbing the driver directory — the ACPI instance id in
    /// the path (`AMDI0101:00`, `AMDI0015:00`, …) is board-specific and must
    /// never be hardcoded.
    pub mode_path: PathBuf,
    /// Current mode as reported by the driver (`frequency` / `cache`).
    pub current_mode: Option<String>,
}

/// Processor topology and frequency-control capabilities.
#[derive(Debug, Clone)]
pub struct Cpu {
    /// Vendor parsed from `/proc/cpuinfo`.
    pub vendor: CpuVendor,
    /// Marketing model string (`model name`).
    pub model: String,
    /// Distinct physical cores.
    pub physical_cores: u32,
    /// Online logical CPUs (threads).
    pub logical_cpus: u32,
    /// True when more than one thread shares a core.
    pub smt: bool,
    /// True when cores advertise differing max frequencies — the signal for
    /// Intel P/E hybrids and AMD's mixed-CCD parts.
    pub hybrid: bool,
    /// `scaling_driver` (`amd-pstate-epp`, `intel_pstate`, `acpi-cpufreq`, …).
    pub scaling_driver: Option<String>,
    /// Governors the kernel will actually accept. On `*-pstate-epp` this is
    /// only `performance` and `powersave`.
    pub available_governors: Vec<String>,
    /// Governor currently set on CPU 0.
    pub current_governor: Option<String>,
    /// Energy Performance Preference values the driver accepts, if any.
    pub available_epp: Vec<String>,
    /// Current EPP on CPU 0.
    pub current_epp: Option<String>,
    /// Contents of `/sys/devices/system/cpu/amd_pstate/status`.
    pub amd_pstate_status: Option<String>,
    /// 3D V-Cache control, if this part has it.
    pub vcache: Option<VCacheDevice>,
}

impl Cpu {
    /// Whether a governor name can actually be written on this machine.
    #[must_use]
    pub fn supports_governor(&self, name: &str) -> bool {
        self.available_governors.iter().any(|g| g == name)
    }

    /// Whether the energy preference is what a power profile sets.
    ///
    /// True for amd-pstate in active mode (`amd-pstate-epp`), where
    /// power-profiles-daemon drives EPP and the governor is only the
    /// `performance`/`powersave` pair. Forcing `performance` there overrides
    /// the profile's choice rather than adding anything to it.
    #[must_use]
    pub fn epp_driven_by_power_profile(&self) -> bool {
        self.scaling_driver.as_deref() == Some("amd-pstate-epp")
            && self.amd_pstate_status.as_deref() == Some("active")
    }
}

// ── GPU ──────────────────────────────────────────────────────────────────────

/// GPU manufacturer, from the PCI vendor id in the DRM device's `uevent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    /// PCI vendor `0x1002`.
    Amd,
    /// PCI vendor `0x10de`.
    Nvidia,
    /// PCI vendor `0x8086`.
    Intel,
    /// Anything else.
    Other,
}

/// One DRM card.
#[derive(Debug, Clone)]
pub struct Gpu {
    /// DRM node name, e.g. `card1`.
    pub card: String,
    /// `/sys/class/drm/<card>/device`.
    pub device_path: PathBuf,
    /// Vendor from PCI id.
    pub vendor: GpuVendor,
    /// `vendor:device` PCI id, e.g. `1002:7590`.
    pub pci_id: String,
    /// Kernel driver bound to the device (`amdgpu`, `nvidia`, `i915`, `xe`).
    pub driver: String,
    /// The card's `hwmon` directory, when it exposes one.
    pub hwmon: Option<PathBuf>,
    /// Connector names currently reporting `connected`.
    pub connected_outputs: Vec<String>,
    /// Total VRAM in bytes, when the driver reports it.
    pub vram_total_bytes: Option<u64>,
    /// True when the card looks discrete rather than an integrated/APU block.
    pub discrete: bool,
    /// `power_dpm_force_performance_level`, when writable by the driver.
    pub dpm_level_path: Option<PathBuf>,
}

impl Gpu {
    /// Read `power_dpm_force_performance_level`, if present.
    #[must_use]
    pub fn dpm_level(&self) -> Option<String> {
        let path = self.dpm_level_path.as_ref()?;
        std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_owned())
    }

    /// Read an integer from this card's hwmon directory.
    #[must_use]
    pub fn hwmon_u64(&self, attr: &str) -> Option<u64> {
        let dir = self.hwmon.as_ref()?;
        std::fs::read_to_string(dir.join(attr))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Current GPU utilisation percentage (`gpu_busy_percent`), AMD only.
    #[must_use]
    pub fn busy_percent(&self) -> Option<u8> {
        std::fs::read_to_string(self.device_path.join("gpu_busy_percent"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
}

// ── Display ──────────────────────────────────────────────────────────────────

/// A connected output.
#[derive(Debug, Clone)]
pub struct Display {
    /// Connector name, e.g. `HDMI-A-1`.
    pub connector: String,
    /// DRM card the connector belongs to.
    pub card: String,
    /// Highest resolution the connector advertises, as `(width, height)`.
    pub max_mode: Option<(u32, u32)>,
    /// Whether the kernel reports the connector as VRR-capable.
    ///
    /// `None` means the `vrr_capable` attribute did not exist — which is common
    /// and must not be read as "no VRR". It means "unknown from sysfs".
    pub vrr_capable: Option<bool>,
}

// ── Machine ──────────────────────────────────────────────────────────────────

/// Physical form factor, from `/sys/class/dmi/id/chassis_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chassis {
    /// Desktop, tower, mini-PC.
    Desktop,
    /// Laptop, notebook, convertible.
    Laptop,
    /// Handheld gaming device.
    Handheld,
    /// Could not be determined.
    Unknown,
}

/// Where the machine is drawing power from right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    /// Mains.
    Ac,
    /// Running on battery.
    Battery,
    /// No battery present, or state unreadable.
    Unknown,
}

/// Display server in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    /// Wayland compositor.
    Wayland,
    /// Xorg / `XWayland` root session.
    X11,
    /// No graphical session.
    Tty,
}

/// Complete hardware snapshot.
#[derive(Debug, Clone)]
pub struct Hardware {
    /// Processor.
    pub cpu: Cpu,
    /// Every DRM card found.
    pub gpus: Vec<Gpu>,
    /// Index into [`Hardware::gpus`] of the card games will render on.
    pub render_gpu: Option<usize>,
    /// Connected outputs across all cards.
    pub displays: Vec<Display>,
    /// Form factor.
    pub chassis: Chassis,
    /// Current power source.
    pub power_source: PowerSource,
    /// Display server.
    pub session: Session,
    /// `uname -r`.
    pub kernel: String,
}

impl Hardware {
    /// Discover everything. Never fails: unreadable areas become `None`/empty.
    #[must_use]
    pub fn detect() -> Self {
        let gpus = detect_gpus();
        let render_gpu = pick_render_gpu(&gpus);
        Self {
            cpu: detect_cpu(),
            displays: detect_displays(),
            gpus,
            render_gpu,
            chassis: detect_chassis(),
            power_source: detect_power_source(),
            session: detect_session(),
            kernel: read_trim("/proc/sys/kernel/osrelease").unwrap_or_default(),
        }
    }

    /// The GPU games render on, if one was identified.
    #[must_use]
    pub fn render_gpu(&self) -> Option<&Gpu> {
        self.render_gpu.and_then(|i| self.gpus.get(i))
    }

    /// True when running on battery — the planner must not max everything out.
    #[must_use]
    pub fn on_battery(&self) -> bool {
        self.power_source == PowerSource::Battery
    }

    /// Highest refresh-capable resolution across connected outputs.
    #[must_use]
    pub fn primary_resolution(&self) -> Option<(u32, u32)> {
        self.displays.iter().find_map(|d| d.max_mode)
    }
}

// ── Detection helpers ────────────────────────────────────────────────────────

fn read_trim<P: AsRef<Path>>(p: P) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn detect_cpu() -> Cpu {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    Cpu {
        vendor: parse_cpu_vendor(&cpuinfo),
        model: parse_cpuinfo_field(&cpuinfo, "model name").unwrap_or_else(|| "Unknown CPU".into()),
        physical_cores: count_physical_cores(&cpuinfo),
        logical_cpus: count_logical_cpus(&cpuinfo),
        smt: read_trim("/sys/devices/system/cpu/smt/active").as_deref() == Some("1"),
        hybrid: detect_hybrid(),
        scaling_driver: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver"),
        available_governors: read_trim(
            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
        )
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default(),
        current_governor: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        available_epp: read_trim(
            "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_available_preferences",
        )
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default(),
        current_epp: read_trim(
            "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference",
        ),
        amd_pstate_status: read_trim("/sys/devices/system/cpu/amd_pstate/status"),
        vcache: detect_vcache(),
    }
}

/// Parse `vendor_id` out of `/proc/cpuinfo`.
#[must_use]
pub fn parse_cpu_vendor(cpuinfo: &str) -> CpuVendor {
    match parse_cpuinfo_field(cpuinfo, "vendor_id").as_deref() {
        Some("AuthenticAMD") => CpuVendor::Amd,
        Some("GenuineIntel") => CpuVendor::Intel,
        _ => CpuVendor::Other,
    }
}

/// Read the first value of a `key : value` field from `/proc/cpuinfo` text.
#[must_use]
pub fn parse_cpuinfo_field(cpuinfo: &str, key: &str) -> Option<String> {
    cpuinfo.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_owned())
    })
}

/// Count distinct `(physical id, core id)` pairs; falls back to logical count.
#[must_use]
pub fn count_physical_cores(cpuinfo: &str) -> u32 {
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut phys = String::new();
    for line in cpuinfo.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "physical id" => v.trim().clone_into(&mut phys),
            "core id" => {
                let pair = (phys.clone(), v.trim().to_owned());
                if !seen.contains(&pair) {
                    seen.push(pair);
                }
            }
            _ => {}
        }
    }
    if seen.is_empty() {
        count_logical_cpus(cpuinfo)
    } else {
        u32::try_from(seen.len()).unwrap_or(u32::MAX)
    }
}

/// Count `processor :` entries.
#[must_use]
pub fn count_logical_cpus(cpuinfo: &str) -> u32 {
    let n = cpuinfo
        .lines()
        .filter(|l| {
            l.split_once(':')
                .is_some_and(|(k, _)| k.trim() == "processor")
        })
        .count();
    u32::try_from(n).unwrap_or(u32::MAX).max(1)
}

/// Detect asymmetric cores by comparing per-CPU `cpuinfo_max_freq`.
///
/// This catches Intel P/E hybrids and AMD parts with differing CCD limits
/// without needing a vendor-specific attribute.
fn detect_hybrid() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/devices/system/cpu") else {
        return false;
    };
    let mut freqs: Vec<u64> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("cpu") || !name[3..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(f) =
            read_trim(e.path().join("cpufreq/cpuinfo_max_freq")).and_then(|s| s.parse::<u64>().ok())
        {
            freqs.push(f);
        }
    }
    let Some(&first) = freqs.first() else {
        return false;
    };
    // Tolerate small per-core binning spread; a real P/E split is far larger.
    freqs.iter().any(|f| f.abs_diff(first) > first / 10)
}

/// Locate the AMD 3D V-Cache control attribute by globbing the driver dir.
fn detect_vcache() -> Option<VCacheDevice> {
    const DRIVER_DIR: &str = "/sys/bus/platform/drivers/amd_x3d_vcache";
    for entry in std::fs::read_dir(DRIVER_DIR).ok()?.flatten() {
        let path = entry.path().join("amd_x3d_mode");
        if path.exists() {
            let current_mode = read_trim(&path);
            return Some(VCacheDevice {
                mode_path: path,
                current_mode,
            });
        }
    }
    None
}

fn detect_gpus() -> Vec<Gpu> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut gpus = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Card nodes are `cardN`; `cardN-CONNECTOR` and `renderDN` are not cards.
        if !is_card_node(&name) {
            continue;
        }
        let device_path = entry.path().join("device");
        let uevent = std::fs::read_to_string(device_path.join("uevent")).unwrap_or_default();
        let pci_id = uevent_field(&uevent, "PCI_ID").unwrap_or_default();
        let driver = uevent_field(&uevent, "DRIVER").unwrap_or_default();
        let slot = uevent_field(&uevent, "PCI_SLOT_NAME").unwrap_or_default();
        let vram_total_bytes =
            read_trim(device_path.join("mem_info_vram_total")).and_then(|s| s.parse::<u64>().ok());
        // A dedicated memory vendor string is only populated for real VRAM;
        // APUs carve their aperture out of system RAM and leave it blank.
        let has_vram_vendor = read_trim(device_path.join("mem_info_vram_vendor")).is_some();
        let vendor = gpu_vendor_from_pci_id(&pci_id);
        gpus.push(Gpu {
            discrete: looks_discrete(vendor, &slot, has_vram_vendor, vram_total_bytes),
            vendor,
            pci_id,
            driver,
            hwmon: find_hwmon(&device_path),
            connected_outputs: connected_outputs_for(&name),
            vram_total_bytes,
            dpm_level_path: {
                let p = device_path.join("power_dpm_force_performance_level");
                p.exists().then_some(p)
            },
            device_path,
            card: name,
        });
    }
    gpus.sort_by(|a, b| a.card.cmp(&b.card));
    gpus
}

/// Whether a GPU is a discrete card rather than an integrated one.
///
/// Each vendor needs its own evidence, because only `amdgpu` publishes its
/// memory in sysfs:
/// - NVIDIA: every NVIDIA GPU on PCI is discrete (Tegra is not on PCI). The
///   proprietary driver exposes no VRAM attributes at all, so a VRAM test
///   would call a GeForce "integrated" and send a hybrid laptop's games to the
///   iGPU.
/// - AMD: dedicated VRAM with a memory vendor (APUs carve theirs out of RAM
///   and leave the vendor blank).
/// - Intel: integrated graphics sit on the root bus (`0000:00:02.0`); an Arc
///   card sits behind a PCIe bridge, on another bus.
#[must_use]
pub fn looks_discrete(
    vendor: GpuVendor,
    pci_slot: &str,
    has_vram_vendor: bool,
    vram: Option<u64>,
) -> bool {
    let big_vram = vram.is_some_and(|v| v > 1 << 30);
    match vendor {
        GpuVendor::Nvidia => true,
        GpuVendor::Amd => has_vram_vendor && big_vram,
        GpuVendor::Intel => pci_slot
            .split(':')
            .nth(1)
            .is_some_and(|bus| !bus.is_empty() && bus != "00"),
        GpuVendor::Other => big_vram,
    }
}

/// True for `card0`, `card12`; false for `card0-DP-1`, `renderD128`, `version`.
#[must_use]
pub fn is_card_node(name: &str) -> bool {
    name.strip_prefix("card")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// Extract `KEY=value` from a sysfs `uevent` blob.
#[must_use]
pub fn uevent_field(uevent: &str, key: &str) -> Option<String> {
    uevent.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k == key).then(|| v.trim().to_owned())
    })
}

/// Map a `vendor:device` PCI id to a [`GpuVendor`].
#[must_use]
pub fn gpu_vendor_from_pci_id(pci_id: &str) -> GpuVendor {
    match pci_id
        .split(':')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("1002") => GpuVendor::Amd,
        Some("10de") => GpuVendor::Nvidia,
        Some("8086") => GpuVendor::Intel,
        _ => GpuVendor::Other,
    }
}

/// First `device/hwmon/hwmonN` directory under a DRM device.
fn find_hwmon(device_path: &Path) -> Option<PathBuf> {
    std::fs::read_dir(device_path.join("hwmon"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("temp1_input").exists() || p.join("freq1_input").exists())
}

/// Connector nodes for `card` whose `status` reads `connected`.
fn connected_outputs_for(card: &str) -> Vec<String> {
    let prefix = format!("{card}-");
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix)?;
            (read_trim(e.path().join("status")).as_deref() == Some("connected"))
                .then(|| rest.to_owned())
        })
        .collect();
    out.sort();
    out
}

/// Choose the card games will render on.
///
/// Preference order, highest first:
/// 1. a discrete card (dedicated VRAM) — on hybrid laptops the dGPU drives no
///    connector at all, so "has outputs" alone would pick the wrong one;
/// 2. among equals, the card with the most VRAM;
/// 3. among equals, a card that actually drives a connected output;
/// 4. the first card, so a single-GPU machine always gets an answer.
#[must_use]
pub fn pick_render_gpu(gpus: &[Gpu]) -> Option<usize> {
    gpus.iter()
        .enumerate()
        .max_by_key(|(_, g)| {
            (
                u8::from(g.discrete),
                g.vram_total_bytes.unwrap_or(0),
                u8::from(!g.connected_outputs.is_empty()),
            )
        })
        .map(|(i, _)| i)
}

fn detect_displays() -> Vec<Display> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_card_node(&name) || !name.starts_with("card") {
            continue;
        }
        let Some((card, connector)) = name.split_once('-') else {
            continue;
        };
        if read_trim(e.path().join("status")).as_deref() != Some("connected") {
            continue;
        }
        out.push(Display {
            connector: connector.to_owned(),
            card: card.to_owned(),
            max_mode: read_max_mode(&e.path().join("modes")),
            // Absent attribute means "sysfs does not say", not "no VRR".
            vrr_capable: read_trim(e.path().join("vrr_capable")).map(|v| v == "1"),
        });
    }
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

/// Largest `WxH` listed in a DRM connector's `modes` file.
fn read_max_mode(modes: &Path) -> Option<(u32, u32)> {
    let content = std::fs::read_to_string(modes).ok()?;
    content
        .lines()
        .filter_map(|l| parse_mode(l.trim()))
        .max_by_key(|(w, h)| u64::from(*w) * u64::from(*h))
}

/// Parse a DRM mode string such as `3440x1440`.
#[must_use]
pub fn parse_mode(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.trim_end_matches('i').parse().ok()?))
}

fn detect_chassis() -> Chassis {
    // SMBIOS chassis types, per DSP0134 §7.4.1.
    match read_trim("/sys/class/dmi/id/chassis_type").as_deref() {
        Some("3" | "4" | "5" | "6" | "7" | "15" | "16" | "17" | "23" | "24") => Chassis::Desktop,
        Some("8" | "9" | "10" | "11" | "12" | "14" | "18" | "21" | "31" | "32") => Chassis::Laptop,
        Some("13" | "30") => Chassis::Handheld,
        _ => Chassis::Unknown,
    }
}

fn detect_power_source() -> PowerSource {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return PowerSource::Unknown;
    };
    let mut saw_battery = false;
    let mut mains_online = None;
    for e in entries.flatten() {
        match read_trim(e.path().join("type")).as_deref() {
            Some("Mains") => {
                if read_trim(e.path().join("online")).as_deref() == Some("1") {
                    mains_online = Some(true);
                } else if mains_online.is_none() {
                    mains_online = Some(false);
                }
            }
            Some("Battery") => saw_battery = true,
            _ => {}
        }
    }
    match (saw_battery, mains_online) {
        // A machine with no battery at all is simply a desktop on mains, which
        // is the same conclusion as a battery machine reporting mains online.
        (false, _) | (true, Some(true)) => PowerSource::Ac,
        (true, Some(false)) => PowerSource::Battery,
        (true, None) => PowerSource::Unknown,
    }
}

fn detect_session() -> Session {
    match std::env::var("XDG_SESSION_TYPE").as_deref() {
        Ok("wayland") => Session::Wayland,
        Ok("x11") => Session::X11,
        _ => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                Session::Wayland
            } else if std::env::var_os("DISPLAY").is_some() {
                Session::X11
            } else {
                Session::Tty
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO: &str = "\
processor\t: 0
vendor_id\t: AuthenticAMD
model name\t: AMD Ryzen 7 5700G with Radeon Graphics
physical id\t: 0
core id\t\t: 0

processor\t: 1
vendor_id\t: AuthenticAMD
model name\t: AMD Ryzen 7 5700G with Radeon Graphics
physical id\t: 0
core id\t\t: 0

processor\t: 2
vendor_id\t: AuthenticAMD
model name\t: AMD Ryzen 7 5700G with Radeon Graphics
physical id\t: 0
core id\t\t: 1
";

    #[test]
    fn parses_vendor_and_model() {
        assert_eq!(parse_cpu_vendor(CPUINFO), CpuVendor::Amd);
        assert_eq!(
            parse_cpuinfo_field(CPUINFO, "model name").as_deref(),
            Some("AMD Ryzen 7 5700G with Radeon Graphics")
        );
    }

    #[test]
    fn intel_and_unknown_vendors() {
        assert_eq!(
            parse_cpu_vendor("vendor_id\t: GenuineIntel"),
            CpuVendor::Intel
        );
        assert_eq!(parse_cpu_vendor("vendor_id\t: Hygon"), CpuVendor::Other);
        assert_eq!(parse_cpu_vendor(""), CpuVendor::Other);
    }

    #[test]
    fn counts_cores_and_threads() {
        // 3 logical CPUs over 2 distinct (physical id, core id) pairs.
        assert_eq!(count_logical_cpus(CPUINFO), 3);
        assert_eq!(count_physical_cores(CPUINFO), 2);
    }

    #[test]
    fn core_count_falls_back_to_logical_when_topology_absent() {
        let minimal = "processor\t: 0\nprocessor\t: 1\n";
        assert_eq!(count_physical_cores(minimal), 2);
    }

    #[test]
    fn logical_cpu_count_is_never_zero() {
        assert_eq!(count_logical_cpus(""), 1);
    }

    #[test]
    fn card_nodes_exclude_connectors_and_render_nodes() {
        assert!(is_card_node("card0"));
        assert!(is_card_node("card12"));
        // These are exactly the entries that made the old telemetry walk abort.
        assert!(!is_card_node("card1-DP-1"));
        assert!(!is_card_node("card1-HDMI-A-1"));
        assert!(!is_card_node("renderD128"));
        assert!(!is_card_node("version"));
        assert!(!is_card_node("card"));
    }

    #[test]
    fn parses_uevent_fields() {
        let uevent = "DRIVER=amdgpu\nPCI_ID=1002:7590\nPCI_SLOT_NAME=0000:03:00.0\n";
        assert_eq!(uevent_field(uevent, "PCI_ID").as_deref(), Some("1002:7590"));
        assert_eq!(uevent_field(uevent, "DRIVER").as_deref(), Some("amdgpu"));
        assert_eq!(uevent_field(uevent, "MISSING"), None);
    }

    #[test]
    fn maps_pci_vendors() {
        assert_eq!(gpu_vendor_from_pci_id("1002:7590"), GpuVendor::Amd);
        assert_eq!(gpu_vendor_from_pci_id("10DE:2684"), GpuVendor::Nvidia);
        assert_eq!(gpu_vendor_from_pci_id("8086:56a0"), GpuVendor::Intel);
        assert_eq!(gpu_vendor_from_pci_id(""), GpuVendor::Other);
    }

    #[test]
    fn parses_drm_modes() {
        assert_eq!(parse_mode("3440x1440"), Some((3440, 1440)));
        assert_eq!(parse_mode("1920x1080i"), Some((1920, 1080)));
        assert_eq!(parse_mode("garbage"), None);
    }

    fn gpu(card: &str, discrete: bool, vram: Option<u64>, outputs: &[&str]) -> Gpu {
        Gpu {
            card: card.into(),
            device_path: PathBuf::from("/dev/null"),
            vendor: GpuVendor::Amd,
            pci_id: String::new(),
            driver: "amdgpu".into(),
            hwmon: None,
            connected_outputs: outputs.iter().map(|s| (*s).to_owned()).collect(),
            vram_total_bytes: vram,
            discrete,
            dpm_level_path: None,
        }
    }

    #[test]
    fn render_gpu_prefers_discrete_over_the_igpu_that_drives_no_output() {
        // Exactly the bench layout: card0 = Cezanne iGPU (512 MiB, no outputs),
        // card1 = RX 9060 XT (16 GiB, all three connectors).
        let gpus = vec![
            gpu("card0", false, Some(536_870_912), &[]),
            gpu(
                "card1",
                true,
                Some(17_095_983_104),
                &["DP-1", "DP-2", "HDMI-A-1"],
            ),
        ];
        assert_eq!(pick_render_gpu(&gpus), Some(1));
    }

    #[test]
    fn render_gpu_prefers_headless_dgpu_on_hybrid_laptops() {
        // NVIDIA offload: the dGPU drives no connector, the iGPU drives them all.
        // Picking "the card with outputs" would be wrong here.
        let gpus = vec![
            gpu("card0", false, Some(268_435_456), &["eDP-1"]),
            gpu("card1", true, Some(8_589_934_592), &[]),
        ];
        assert_eq!(pick_render_gpu(&gpus), Some(1));
    }

    #[test]
    fn discrete_comes_from_each_vendors_own_evidence() {
        // The lab laptop: i915 at 0000:00:02.0, GTX 1050 Ti Mobile on the
        // proprietary driver at 0000:01:00.0 with no VRAM attributes.
        assert!(looks_discrete(GpuVendor::Nvidia, "0000:01:00.0", false, None));
        assert!(!looks_discrete(GpuVendor::Intel, "0000:00:02.0", false, None));
        // Arc behind a PCIe bridge.
        assert!(looks_discrete(GpuVendor::Intel, "0000:03:00.0", false, None));
        // RX 9060 XT vs the Cezanne iGPU's 512 MiB carve-out.
        assert!(looks_discrete(
            GpuVendor::Amd,
            "0000:03:00.0",
            true,
            Some(17_095_983_104)
        ));
        assert!(!looks_discrete(
            GpuVendor::Amd,
            "0000:07:00.0",
            false,
            Some(536_870_912)
        ));
        assert!(!looks_discrete(GpuVendor::Intel, "", false, None));
    }

    #[test]
    fn render_gpu_single_card_always_resolves() {
        let gpus = vec![gpu("card0", false, None, &["eDP-1"])];
        assert_eq!(pick_render_gpu(&gpus), Some(0));
    }

    #[test]
    fn render_gpu_none_without_cards() {
        assert_eq!(pick_render_gpu(&[]), None);
    }

    #[test]
    fn governor_support_is_checked_against_the_real_list() {
        let cpu = Cpu {
            vendor: CpuVendor::Amd,
            model: String::new(),
            physical_cores: 8,
            logical_cpus: 16,
            smt: true,
            hybrid: false,
            scaling_driver: Some("amd-pstate-epp".into()),
            // amd-pstate-epp offers only these two.
            available_governors: vec!["performance".into(), "powersave".into()],
            current_governor: Some("performance".into()),
            available_epp: Vec::new(),
            current_epp: Some("performance".into()),
            amd_pstate_status: Some("active".into()),
            vcache: None,
        };
        assert!(cpu.supports_governor("performance"));
        assert!(cpu.supports_governor("powersave"));
        assert!(!cpu.supports_governor("schedutil"));
        assert!(!cpu.supports_governor("ondemand"));
    }

    #[test]
    fn detect_runs_on_this_machine() {
        // Smoke test: detection must never panic on a real system.
        let hw = Hardware::detect();
        assert!(hw.cpu.logical_cpus >= 1);
    }
}
