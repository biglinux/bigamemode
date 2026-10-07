//! Hardware discovery.
//!
//! Everything here is *observation only* — no file in this module ever writes to
//! the system. The result is the ground truth the Booster planner reasons over,
//! so a wrong answer here silently becomes a wrong optimization later. Each
//! field therefore records what was actually read, and uses `Option` rather than
//! a guessed default whenever the system did not tell us.

use std::path::{Path, PathBuf};

use crate::text::{N_, Text};

/// [`Cpu::model`] when `/proc/cpuinfo` names none. The UI shows it translated.
pub const UNKNOWN_CPU: &str = N_("Unknown CPU");

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
    /// True for amd-pstate in active mode (`amd-pstate-epp`) and for
    /// `intel_pstate` in active mode with HWP, where power-profiles-daemon
    /// drives EPP and the governor is only the `performance`/`powersave`
    /// pair. Forcing `performance` there overrides the profile's choice
    /// rather than adding anything to it.
    #[must_use]
    pub fn epp_driven_by_power_profile(&self) -> bool {
        match self.scaling_driver.as_deref() {
            Some("amd-pstate-epp") => self.amd_pstate_status.as_deref() == Some("active"),
            // Passive mode is `intel_cpufreq`; only HWP publishes an EPP.
            Some("intel_pstate") => self.current_epp.is_some(),
            _ => false,
        }
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

/// Drivers of firmware framebuffers: the boot display handed over by the
/// firmware, kept until (or when no) real GPU driver takes over.
const FIRMWARE_FRAMEBUFFERS: &[&str] = &[
    "simple-framebuffer",
    "simpledrm",
    "efi-framebuffer",
    "efidrm",
    "vesa-framebuffer",
    "vesadrm",
    "ofdrm",
];

/// Drivers that reserve a device for a virtual machine.
const HELD_FOR_GUESTS: &[&str] = &["vfio-pci", "pci-stub"];

/// DRM drivers of 2D display controllers: BMC chips and emulated VGA.
const DISPLAY_ONLY: &[&str] = &[
    "ast",
    "mgag200",
    "bochs-drm",
    "bochs",
    "cirrus",
    "cirrus-qemu",
    "hibmc-drm",
];

/// One DRM card.
#[derive(Debug, Clone)]
pub struct Gpu {
    /// DRM node name, e.g. `card1`; empty for a display controller no DRM
    /// card stands for (found on PCI).
    pub card: String,
    /// `/sys/class/drm/<card>/device`.
    pub device_path: PathBuf,
    /// Vendor from PCI id.
    pub vendor: GpuVendor,
    /// `vendor:device` PCI id, e.g. `1002:7590`.
    pub pci_id: String,
    /// PCI address, e.g. `0000:01:00.0`; empty when not on PCI.
    pub pci_slot: String,
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
    /// How the GPU is named in reports: its DRM card (`card1`), or its PCI
    /// address when it has no DRM node.
    #[must_use]
    pub fn node(&self) -> &str {
        if self.card.is_empty() {
            &self.pci_slot
        } else {
            &self.card
        }
    }

    /// Whether the host can render on this device at all.
    ///
    /// A firmware framebuffer (simpledrm and its kin) is no GPU, and a device
    /// bound to no driver, or held for a virtual machine (`vfio-pci`,
    /// `pci-stub`), cannot be used by the host. They stay in the list so the
    /// reports name them, and are never chosen for games.
    #[must_use]
    pub fn can_render(&self) -> bool {
        !self.driver.is_empty()
            && !FIRMWARE_FRAMEBUFFERS.contains(&self.driver.as_str())
            && !HELD_FOR_GUESTS.contains(&self.driver.as_str())
    }

    /// Whether this is a 2D display controller with no 3D engine: a server's
    /// BMC chip, or a VM's emulated VGA.
    #[must_use]
    pub fn display_only(&self) -> bool {
        DISPLAY_ONLY.contains(&self.driver.as_str())
    }

    /// Whether the GPU drives a connected output someone looks at.
    ///
    /// A firmware framebuffer reports its stand-in connector (`Unknown-1`)
    /// as connected whatever is plugged in, and a BMC's virtual connector
    /// (`Virtual-1` on ast) is always connected for the remote console.
    #[must_use]
    pub fn drives_display(&self) -> bool {
        if FIRMWARE_FRAMEBUFFERS.contains(&self.driver.as_str()) {
            return false;
        }
        let bmc = matches!(self.driver.as_str(), "ast" | "mgag200");
        self.connected_outputs
            .iter()
            .any(|c| !(bmc && c.starts_with("Virtual-")))
    }

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
    /// The connector's preferred resolution, the first it lists (the
    /// monitor's native one), as `(width, height)`.
    pub max_mode: Option<(u32, u32)>,
    /// Whether the kernel reports the connector as VRR-capable.
    ///
    /// `None` means the `vrr_capable` attribute did not exist, which is the
    /// rule: the kernel publishes VRR capability as a DRM connector property,
    /// not in sysfs. It means "unknown", never "no VRR".
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
        model: cpu_model(&cpuinfo).unwrap_or_else(|| UNKNOWN_CPU.into()),
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

/// The processor's name from `/proc/cpuinfo`: `model name` on x86, the
/// keys other kernels use elsewhere. An empty value is no name — some
/// hypervisors and early kernels for a new part leave it blank.
#[must_use]
pub fn cpu_model(cpuinfo: &str) -> Option<String> {
    ["model name", "cpu model", "Model", "Processor", "Hardware"]
        .into_iter()
        .find_map(|key| parse_cpuinfo_field(cpuinfo, key).filter(|v| !v.is_empty()))
}

/// A processor's name as people read it: the model without trademark
/// signs, the "CPU"/"Processor"/"N-Core" filler, the base clock and the
/// integrated graphics (named on their own), with single spaces.
///
/// `Intel(R) Core(TM) i7-12700K CPU @ 3.60GHz` → `Intel Core i7-12700K`;
/// `AMD Ryzen 7 7840HS w/ Radeon 780M Graphics` → `AMD Ryzen 7 7840HS`;
/// `AMD Ryzen 9 7950X 16-Core Processor` → `AMD Ryzen 9 7950X`.
/// A name that is only filler is kept as it was.
#[must_use]
pub fn cpu_display_name(model: &str) -> String {
    let mut name = model.to_owned();
    for mark in ["(R)", "(r)", "(TM)", "(tm)", "®", "™"] {
        name = name.replace(mark, "");
    }
    // The base clock, and the graphics named after the processor.
    for cut in [" @ ", " with Radeon", " w/ Radeon", " with AMD Radeon"] {
        if let Some(i) = name.find(cut) {
            name.truncate(i);
        }
    }
    // "CPU" is filler in the makers' own strings, and a word elsewhere
    // (`QEMU Virtual CPU version 2.5+`).
    let maker = name.contains("Intel") || name.contains("AMD");
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|w| {
            let filler = w.eq_ignore_ascii_case("processor")
                || w.to_ascii_lowercase().ends_with("-core")
                || (maker && *w == "CPU");
            !filler
        })
        .collect();
    if words.is_empty() {
        model.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        words.join(" ")
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
pub(crate) fn detect_vcache() -> Option<VCacheDevice> {
    const DRIVER_DIR: &str = "/sys/bus/platform/drivers/amd_x3d_vcache";
    // The ACPI instance id in the path (`AMDI0101:00`, `AMDI0015:00`, …) is
    // board-specific: the attribute is found by listing the driver directory.
    for entry in std::fs::read_dir(DRIVER_DIR).ok()?.flatten() {
        let path = entry.path().join("amd_x3d_mode");
        if path.exists() {
            return Some(VCacheDevice {
                current_mode: read_trim(&path),
            });
        }
    }
    None
}

fn detect_gpus() -> Vec<Gpu> {
    detect_gpus_in(Path::new(DRM_CLASS), Path::new("/sys/bus/pci/devices"))
}

const DRM_CLASS: &str = "/sys/class/drm";

/// Every GPU under `drm` (`/sys/class/drm`), then the display controllers
/// under `pci` (`/sys/bus/pci/devices`) no card stands for.
fn detect_gpus_in(drm: &Path, pci: &Path) -> Vec<Gpu> {
    let Ok(entries) = std::fs::read_dir(drm) else {
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
        let driver = uevent_field(&uevent, "DRIVER").unwrap_or_default();
        let mut pci_id = uevent_field(&uevent, "PCI_ID").unwrap_or_default();
        let mut slot = uevent_field(&uevent, "PCI_SLOT_NAME").unwrap_or_default();
        // virtio-gpu's card hangs off `virtio0`, a child of the PCI device;
        // without the address that device would be listed again as a second
        // GPU with no card.
        if slot.is_empty() {
            if let Some(parent) = display_pci_ancestor(&device_path) {
                if pci_id.is_empty() {
                    pci_id = pci_id_of(&parent).unwrap_or_default();
                }
                slot = parent
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
        }
        let vram_total_bytes =
            read_trim(device_path.join("mem_info_vram_total")).and_then(|s| s.parse::<u64>().ok());
        // A dedicated memory vendor string is only populated for real VRAM;
        // APUs carve their aperture out of system RAM and leave it blank.
        let has_vram_vendor = read_trim(device_path.join("mem_info_vram_vendor")).is_some();
        let hwmon = find_hwmon(&device_path);
        // amdgpu publishes the northbridge rail (`vddnb`) on APUs only; the
        // label is a constant string, read without waking the device.
        let apu_rail = hwmon
            .as_ref()
            .filter(|_| driver == "amdgpu")
            .map(|h| read_trim(h.join("in1_label")).as_deref() == Some("vddnb"));
        let vendor = gpu_vendor_from_pci_id(&pci_id);
        let discrete = looks_discrete(
            vendor,
            &driver,
            &slot,
            has_vram_vendor,
            apu_rail,
            vram_total_bytes,
        );
        gpus.push(Gpu {
            discrete,
            vendor,
            pci_id,
            pci_slot: slot,
            driver,
            hwmon,
            connected_outputs: connected_outputs_for(drm, &name),
            vram_total_bytes,
            dpm_level_path: {
                let p = device_path.join("power_dpm_force_performance_level");
                p.exists().then_some(p)
            },
            device_path,
            card: name,
        });
    }
    // By number: `card10` after `card2`.
    gpus.sort_by_key(|g| card_number(&g.card));
    gpus.extend(pci_display_devices(pci, &gpus));
    gpus
}

/// `2` for `card2`; cards without a number sort last.
fn card_number(card: &str) -> u32 {
    card.strip_prefix("card")
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Whether `name` is a PCI address, `0000:01:00.0`.
fn is_pci_address(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 12
        && b[4] == b':'
        && b[7] == b':'
        && b[10] == b'.'
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7 | 10) || c.is_ascii_hexdigit())
}

/// The PCI display controller a DRM device sits on, when the device is not
/// the PCI function itself: the nearest PCI address among its ancestors,
/// if that is a display controller (class 0x03). A USB display adapter's
/// nearest PCI device is its USB controller, which is no GPU.
fn display_pci_ancestor(device_path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(device_path).ok()?;
    let pci = real.ancestors().find(|a| {
        a.file_name()
            .is_some_and(|n| is_pci_address(&n.to_string_lossy()))
    })?;
    read_trim(pci.join("class"))
        .is_some_and(|c| c.starts_with("0x03"))
        .then(|| pci.to_path_buf())
}

/// `VVVV:DDDD` from a PCI device directory's `vendor` and `device`.
fn pci_id_of(pci_device: &Path) -> Option<String> {
    let id = |f: &str| {
        read_trim(pci_device.join(f)).map(|v| v.trim_start_matches("0x").to_ascii_uppercase())
    };
    Some(format!("{}:{}", id("vendor")?, id("device")?))
}

/// Display controllers on PCI that no DRM card stands for: an NVIDIA GPU
/// whose driver runs without `nvidia-drm`, or a card no driver is bound to
/// (or one held for a virtual machine). They are still the machine's GPUs,
/// named on Home and in the reports; only one a rendering driver is bound
/// to can be chosen for games ([`Gpu::can_render`]).
fn pci_display_devices(pci: &Path, known: &[Gpu]) -> Vec<Gpu> {
    let Ok(entries) = std::fs::read_dir(pci) else {
        return Vec::new();
    };
    let mut out: Vec<Gpu> = entries
        .flatten()
        .filter_map(|e| {
            let slot = e.file_name().to_string_lossy().into_owned();
            // PCI class 0x03: VGA, XGA, 3D (an Optimus GPU is a 3D controller).
            let class = read_trim(e.path().join("class"))?;
            if !class.starts_with("0x03") || known.iter().any(|g| g.pci_slot == slot) {
                return None;
            }
            let pci_id = pci_id_of(&e.path())?;
            let vendor = gpu_vendor_from_pci_id(&pci_id);
            let driver = std::fs::read_link(e.path().join("driver"))
                .ok()
                .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let device_path = e.path();
            Some(Gpu {
                card: String::new(),
                discrete: looks_discrete(vendor, &driver, &slot, false, None, None),
                vendor,
                pci_id,
                hwmon: find_hwmon(&device_path),
                device_path,
                pci_slot: slot,
                driver,
                connected_outputs: Vec::new(),
                vram_total_bytes: None,
                dpm_level_path: None,
            })
        })
        .collect();
    out.sort_by(|a, b| a.pci_slot.cmp(&b.pci_slot));
    out
}

/// Whether a GPU is a discrete card rather than an integrated one.
///
/// Each vendor needs its own evidence, because only `amdgpu` publishes its
/// memory in sysfs:
/// - NVIDIA: every NVIDIA GPU on PCI is discrete (Tegra is not on PCI). The
///   proprietary driver exposes no VRAM attributes at all, so a VRAM test
///   would call an NVIDIA card "integrated" and send a hybrid laptop's games to the
///   iGPU.
/// - AMD on `amdgpu`: dedicated VRAM with a memory vendor, or a hwmon
///   without the APU's northbridge rail (`apu_rail`: `Some(false)`). Older
///   boards (GCN up to Polaris) publish no memory vendor; APUs carve their
///   memory out of RAM, publish none either, and have the rail.
/// - AMD on `radeon`, and Intel: integrated graphics sit on the root bus
///   (`0000:00:01.0` on pre-Zen APUs, `0000:00:02.0` on Intel); a card sits
///   behind a PCI Express bridge, on another bus. Zen APUs sit off the root
///   bus, which is why `amdgpu` needs its own evidence.
#[must_use]
pub fn looks_discrete(
    vendor: GpuVendor,
    driver: &str,
    pci_slot: &str,
    has_vram_vendor: bool,
    apu_rail: Option<bool>,
    vram: Option<u64>,
) -> bool {
    let off_root_bus = pci_slot
        .split(':')
        .nth(1)
        .is_some_and(|bus| !bus.is_empty() && bus != "00");
    match vendor {
        GpuVendor::Nvidia => true,
        GpuVendor::Amd if driver == "radeon" => off_root_bus,
        GpuVendor::Amd => has_vram_vendor || apu_rail == Some(false),
        GpuVendor::Intel => off_root_bus,
        GpuVendor::Other => vram.is_some_and(|v| v > 1 << 30),
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
fn connected_outputs_for(drm: &Path, card: &str) -> Vec<String> {
    let prefix = format!("{card}-");
    let Ok(entries) = std::fs::read_dir(drm) else {
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
/// Only a GPU the host can render on is a candidate ([`Gpu::can_render`]).
/// Preference order, highest first:
/// 1. a GPU with a 3D engine over a 2D display controller (a BMC, emulated
///    VGA);
/// 2. a discrete card — on hybrid laptops the dGPU drives no connector at
///    all, so "has outputs" alone would pick the wrong one;
/// 3. among equals, the card with the most VRAM, when every one of them
///    reports it: NVIDIA's driver, i915 and xe publish none, and an unknown
///    amount is not zero;
/// 4. among equals, a card that drives a connected output;
/// 5. the first card, so a single-GPU machine always gets an answer.
#[must_use]
pub fn pick_render_gpu(gpus: &[Gpu]) -> Option<usize> {
    let rank = |g: &Gpu| (u8::from(!g.display_only()), u8::from(g.discrete));
    let top = gpus.iter().filter(|g| g.can_render()).map(rank).max()?;
    let group: Vec<usize> = (0..gpus.len())
        .filter(|&i| gpus[i].can_render() && rank(&gpus[i]) == top)
        .collect();
    let vram_known = group.iter().all(|&i| gpus[i].vram_total_bytes.is_some());
    // `max_by_key` keeps the last of equals; reversed, the first card wins.
    group.into_iter().rev().max_by_key(|&i| {
        let g = &gpus[i];
        (
            g.vram_total_bytes.filter(|_| vram_known),
            u8::from(g.drives_display()),
        )
    })
}

fn detect_displays() -> Vec<Display> {
    detect_displays_in(Path::new(DRM_CLASS))
}

fn detect_displays_in(drm: &Path) -> Vec<Display> {
    let Ok(entries) = std::fs::read_dir(drm) else {
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
            max_mode: read_preferred_mode(&e.path().join("modes")),
            // Absent attribute means "sysfs does not say", not "no VRR".
            vrr_capable: read_trim(e.path().join("vrr_capable")).map(|v| v == "1"),
        });
    }
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

// ── Hybrid graphics ──────────────────────────────────────────────────────────

/// How a program is sent to render on the discrete GPU of a machine whose
/// display is driven by another GPU (PRIME render offload).
///
/// Each driver stack has its own switch, and the wrong one half-works: on the
/// NVIDIA proprietary driver `DRI_PRIME=1` gives OpenGL through zink on top of
/// NVIDIA's Vulkan rather than NVIDIA's own OpenGL. DXVK and VKD3D-Proton
/// pick the discrete GPU by themselves; a native game renders on the GPU that
/// drives the display unless told otherwise, OpenGL and Vulkan alike: on the
/// GTX 1050 Ti laptop `SuperTuxKart`'s Vulkan renderer took the HD 630, listed
/// first, until the Optimus layer filter put the GTX first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offload {
    /// NVIDIA proprietary driver: libglvnd's vendor selection plus the
    /// Optimus Vulkan layer filter.
    Nvidia,
    /// A Mesa driver (amdgpu, nouveau, i915/xe dGPU): `DRI_PRIME` naming the
    /// card by PCI address, unambiguous with more than two GPUs.
    DriPrime(String),
}

impl Offload {
    /// The environment that sends a program to the discrete GPU.
    #[must_use]
    pub fn env(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Nvidia => vec![
                ("__NV_PRIME_RENDER_OFFLOAD", "1".to_owned()),
                ("__GLX_VENDOR_LIBRARY_NAME", "nvidia".to_owned()),
                ("__VK_LAYER_NV_optimus", "NVIDIA_only".to_owned()),
            ],
            Self::DriPrime(tag) => vec![("DRI_PRIME", tag.clone())],
        }
    }

    /// Short name of the mechanism, for reports: words to translate, or the
    /// variable's name as it is.
    #[must_use]
    pub fn label(&self) -> Text {
        match self {
            Self::Nvidia => Text::plain(N_("NVIDIA PRIME render offload")),
            Self::DriPrime(_) => Text::raw("DRI_PRIME"),
        }
    }
}

/// Whether games must be offloaded to `gpus[render]`, and how.
///
/// Offload applies when the games' GPU drives no connected output while
/// another GPU does: the laptop layout, where the integrated GPU owns the
/// panel. A discrete card that drives a monitor itself needs nothing, and a
/// firmware framebuffer or a BMC's remote console is no display to offload
/// for ([`Gpu::drives_display`]).
#[must_use]
pub fn offload_for(gpus: &[Gpu], render: usize) -> Option<Offload> {
    let g = gpus.get(render)?;
    let another_drives_display = gpus
        .iter()
        .enumerate()
        .any(|(i, o)| i != render && o.drives_display());
    if !g.can_render() || g.drives_display() || !another_drives_display {
        return None;
    }
    if g.driver == "nvidia" {
        return Some(Offload::Nvidia);
    }
    if g.pci_slot.is_empty() {
        return None;
    }
    // Mesa's form: `pci-0000_03_00_0`.
    Some(Offload::DriPrime(format!(
        "pci-{}",
        g.pci_slot.replace([':', '.'], "_")
    )))
}

/// The mode a DRM connector lists first: the monitor's preferred one, its
/// native resolution. The largest listed is often a mode the monitor only
/// accepts and scales (4096x2160 on a 2560x1080 panel).
fn read_preferred_mode(modes: &Path) -> Option<(u32, u32)> {
    preferred_mode(&std::fs::read_to_string(modes).ok()?)
}

/// The first `WxH` of a connector's `modes` text.
fn preferred_mode(modes: &str) -> Option<(u32, u32)> {
    modes.lines().find_map(|l| parse_mode(l.trim()))
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

/// The display server of this session, from the environment alone.
#[must_use]
pub fn detect_session() -> Session {
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
    fn a_blank_model_name_is_no_name() {
        assert_eq!(
            cpu_model(CPUINFO).as_deref(),
            Some("AMD Ryzen 7 5700G with Radeon Graphics")
        );
        assert_eq!(cpu_model("processor\t: 0\nmodel name\t: \n"), None);
        assert_eq!(
            cpu_model("processor\t: 0\nmodel\t\t: 151\n").as_deref(),
            None,
            "x86's numeric `model` is not a name"
        );
        assert_eq!(
            cpu_model("Processor\t: ARMv7 Processor rev 4 (v7l)\n").as_deref(),
            Some("ARMv7 Processor rev 4 (v7l)")
        );
    }

    #[test]
    fn processor_names_lose_the_filler_and_keep_the_model() {
        for (model, shown) in [
            (
                "AMD Ryzen 7 5700G with Radeon Graphics",
                "AMD Ryzen 7 5700G",
            ),
            (
                "Intel(R) Core(TM) i7-12700K CPU @ 3.60GHz",
                "Intel Core i7-12700K",
            ),
            (
                "13th Gen Intel(R) Core(TM) i7-13700H",
                "13th Gen Intel Core i7-13700H",
            ),
            ("Intel(R) Core(TM) Ultra 7 155H", "Intel Core Ultra 7 155H"),
            ("AMD Ryzen 9 7950X 16-Core Processor", "AMD Ryzen 9 7950X"),
            (
                "AMD Ryzen 7 7840HS w/ Radeon 780M Graphics",
                "AMD Ryzen 7 7840HS",
            ),
            (
                "AMD Ryzen 5 5600X 6-Core Processor             ",
                "AMD Ryzen 5 5600X",
            ),
            ("AMD FX(tm)-8350 Eight-Core Processor", "AMD FX-8350"),
            (
                "Intel(R) Celeron(R) CPU N3350 @ 1.10GHz",
                "Intel Celeron N3350",
            ),
            (
                "Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz",
                "Intel Xeon E5-2680 v4",
            ),
            (
                "Intel(R) Core(TM)2 Duo CPU     E8400  @ 3.00GHz",
                "Intel Core2 Duo E8400",
            ),
            (
                "QEMU Virtual CPU version 2.5+",
                "QEMU Virtual CPU version 2.5+",
            ),
            (
                "AMD Athlon Silver 3050U with Radeon Graphics",
                "AMD Athlon Silver 3050U",
            ),
        ] {
            assert_eq!(cpu_display_name(model), shown, "{model}");
        }
        assert!(!cpu_display_name("Intel(R)  Core(TM)  i5").contains("  "));
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
        // Connector nodes sit beside card nodes in /sys/class/drm and are not
        // cards.
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
            pci_slot: String::new(),
            driver: "amdgpu".into(),
            hwmon: None,
            connected_outputs: outputs.iter().map(|s| (*s).to_owned()).collect(),
            vram_total_bytes: vram,
            discrete,
            dpm_level_path: None,
        }
    }

    fn hybrid(driver: &str, dgpu_outputs: &[&str]) -> Vec<Gpu> {
        let mut dgpu = gpu("card0", true, None, dgpu_outputs);
        dgpu.driver = driver.into();
        dgpu.pci_slot = "0000:01:00.0".into();
        let mut igpu = gpu("card1", false, None, &["eDP-1"]);
        igpu.driver = "i915".into();
        igpu.pci_slot = "0000:00:02.0".into();
        vec![dgpu, igpu]
    }

    #[test]
    fn a_laptop_dgpu_is_offloaded_by_its_own_driver_stack() {
        // The lab laptop: GTX 1050 Ti with no panel, HD 630 on eDP.
        let nv = hybrid("nvidia", &[]);
        let o = offload_for(&nv, 0).unwrap();
        assert_eq!(o, Offload::Nvidia);
        assert!(
            o.env()
                .contains(&("__NV_PRIME_RENDER_OFFLOAD", "1".to_owned()))
        );
        assert!(
            !o.env().iter().any(|(k, _)| *k == "DRI_PRIME"),
            "zink, not NVIDIA's GL"
        );
        // The same laptop with an AMD dGPU: Mesa's DRI_PRIME, by address.
        let amd = hybrid("amdgpu", &[]);
        assert_eq!(
            offload_for(&amd, 0),
            Some(Offload::DriPrime("pci-0000_01_00_0".into()))
        );
    }

    #[test]
    fn no_offload_where_the_games_gpu_drives_a_display_or_is_alone() {
        // External monitor on the dGPU: it renders and presents itself.
        assert_eq!(offload_for(&hybrid("nvidia", &["HDMI-A-1"]), 0), None);
        // The integrated GPU as the games' GPU needs nothing either.
        assert_eq!(offload_for(&hybrid("nvidia", &[]), 1), None);
        // One GPU.
        assert_eq!(offload_for(&[gpu("card0", true, None, &["DP-1"])], 0), None);
    }

    #[test]
    fn render_gpu_prefers_discrete_over_the_igpu_that_drives_no_output() {
        // A common desktop layout: card0 = Cezanne iGPU (512 MiB, no outputs),
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
        assert!(looks_discrete(
            GpuVendor::Nvidia,
            "nvidia",
            "0000:01:00.0",
            false,
            None,
            None
        ));
        assert!(!looks_discrete(
            GpuVendor::Intel,
            "i915",
            "0000:00:02.0",
            false,
            None,
            None
        ));
        // Arc behind a PCIe bridge.
        assert!(looks_discrete(
            GpuVendor::Intel,
            "xe",
            "0000:03:00.0",
            false,
            None,
            None
        ));
        // RX 9060 XT vs the Cezanne iGPU's 512 MiB carve-out, which sits off
        // the root bus and has the northbridge rail.
        assert!(looks_discrete(
            GpuVendor::Amd,
            "amdgpu",
            "0000:03:00.0",
            true,
            Some(false),
            Some(17_095_983_104)
        ));
        assert!(!looks_discrete(
            GpuVendor::Amd,
            "amdgpu",
            "0000:0a:00.0",
            false,
            Some(true),
            Some(536_870_912)
        ));
        assert!(!looks_discrete(
            GpuVendor::Intel,
            "i915",
            "",
            false,
            None,
            None
        ));
    }

    #[test]
    fn amd_boards_without_a_memory_vendor_are_told_apart_from_apus() {
        // An RX 580 on amdgpu publishes no memory vendor; its hwmon has no
        // northbridge rail.
        assert!(looks_discrete(
            GpuVendor::Amd,
            "amdgpu",
            "0000:01:00.0",
            false,
            Some(false),
            Some(8 << 30)
        ));
        // An APU whose firmware reserves 4 GiB is still an APU.
        assert!(!looks_discrete(
            GpuVendor::Amd,
            "amdgpu",
            "0000:c4:00.0",
            false,
            Some(true),
            Some(4 << 30)
        ));
        // `radeon` publishes neither: a Kaveri APU sits on the root bus, an
        // HD 7850 behind a bridge.
        assert!(!looks_discrete(
            GpuVendor::Amd,
            "radeon",
            "0000:00:01.0",
            false,
            None,
            None
        ));
        assert!(looks_discrete(
            GpuVendor::Amd,
            "radeon",
            "0000:01:00.0",
            false,
            None,
            None
        ));
    }

    #[test]
    fn an_rtx_driving_the_monitor_beats_a_spare_radeon_whose_vram_is_known() {
        // NVIDIA's driver publishes no VRAM: an unknown is not 0 bytes, and
        // the 4 GiB Radeon left in the second slot must not win on it.
        let mut rtx = gpu("card0", true, None, &["DP-1"]);
        rtx.driver = "nvidia".into();
        rtx.vendor = GpuVendor::Nvidia;
        rtx.pci_slot = "0000:01:00.0".into();
        let mut radeon = gpu("card1", true, Some(4 << 30), &[]);
        radeon.pci_slot = "0000:04:00.0".into();
        let gpus = vec![rtx, radeon];
        assert_eq!(pick_render_gpu(&gpus), Some(0));
        assert_eq!(offload_for(&gpus, 0), None);
        // Two cards that both report VRAM still compare on it.
        let gpus = vec![
            gpu("card0", true, Some(4 << 30), &["DP-1"]),
            gpu("card1", true, Some(16 << 30), &[]),
        ];
        assert_eq!(pick_render_gpu(&gpus), Some(1));
    }

    #[test]
    fn the_first_of_equal_cards_renders() {
        let gpus = vec![
            gpu("card0", true, Some(8 << 30), &["DP-1"]),
            gpu("card1", true, Some(8 << 30), &["DP-2"]),
        ];
        assert_eq!(pick_render_gpu(&gpus), Some(0));
        let headless = vec![gpu("card0", true, None, &[]), gpu("card1", true, None, &[])];
        assert_eq!(pick_render_gpu(&headless), Some(0));
    }

    #[test]
    fn a_firmware_framebuffer_is_neither_a_render_gpu_nor_a_display() {
        // An NVIDIA desktop with `nvidia-drm.modeset=0`: simpledrm survives
        // with its stand-in connector, the GeForce has no connector at all.
        let mut fb = gpu("card0", false, None, &["Unknown-1"]);
        fb.driver = "simple-framebuffer".into();
        fb.vendor = GpuVendor::Other;
        let mut nv = gpu("card1", true, None, &[]);
        nv.driver = "nvidia".into();
        nv.vendor = GpuVendor::Nvidia;
        nv.pci_slot = "0000:01:00.0".into();
        let gpus = vec![fb, nv];
        assert_eq!(pick_render_gpu(&gpus), Some(1));
        assert_eq!(offload_for(&gpus, 1), None, "no PRIME on a single GPU");
        // Alone, it is still no GPU to render on.
        assert_eq!(pick_render_gpu(&gpus[..1]), None);
    }

    #[test]
    fn a_bmc_console_is_no_display_to_offload_for() {
        // A server board: ast with its always-connected remote console, a
        // GeForce with no connector (no nvidia-drm modeset).
        let mut bmc = gpu("card0", false, None, &["Virtual-1"]);
        bmc.driver = "ast".into();
        bmc.vendor = GpuVendor::Other;
        let mut nv = gpu("card1", true, None, &[]);
        nv.driver = "nvidia".into();
        nv.vendor = GpuVendor::Nvidia;
        let gpus = vec![bmc.clone(), nv];
        assert_eq!(pick_render_gpu(&gpus), Some(1));
        assert_eq!(offload_for(&gpus, 1), None);
        // A monitor on the BMC's VGA port is a display.
        let mut vga = bmc.clone();
        vga.connected_outputs.push("VGA-1".into());
        let gpus = vec![vga, gpus[1].clone()];
        assert_eq!(offload_for(&gpus, 1), Some(Offload::Nvidia));
        // A 2D controller loses to a GPU with a 3D engine, outputs or not.
        let mut igpu = gpu("card1", false, None, &[]);
        igpu.driver = "i915".into();
        let mut vga_bmc = bmc;
        vga_bmc.connected_outputs = vec!["VGA-1".into()];
        assert_eq!(pick_render_gpu(&[vga_bmc, igpu]), Some(1));
    }

    #[test]
    fn a_gpu_held_for_a_guest_or_without_a_driver_is_never_the_games_gpu() {
        // A VFIO host: the APU drives the display, the GeForce is reserved
        // for a virtual machine.
        let mut apu = gpu("card0", false, Some(512 << 20), &["HDMI-A-1"]);
        apu.pci_slot = "0000:0a:00.0".into();
        for driver in ["vfio-pci", "pci-stub", ""] {
            let mut nv = gpu("", true, None, &[]);
            nv.driver = driver.into();
            nv.vendor = GpuVendor::Nvidia;
            nv.pci_slot = "0000:01:00.0".into();
            let gpus = vec![apu.clone(), nv];
            assert_eq!(pick_render_gpu(&gpus), Some(0), "{driver:?}");
            assert_eq!(offload_for(&gpus, 1), None, "{driver:?}");
        }
    }

    #[test]
    fn the_preferred_mode_is_the_first_listed_not_the_largest() {
        // DP-2 of the reference desktop: a 2560x1080 panel that also accepts
        // a 4096x2160 signal.
        assert_eq!(
            preferred_mode("2560x1080\n4096x2160\n2560x1080\n"),
            Some((2560, 1080))
        );
        assert_eq!(preferred_mode(""), None);
    }

    /// A fake sysfs: `/sys/devices`, `/sys/class/drm` and
    /// `/sys/bus/pci/devices`, linked as the kernel links them.
    struct Sysfs {
        dir: tempfile::TempDir,
    }

    impl Sysfs {
        fn new() -> Self {
            let s = Self {
                dir: tempfile::tempdir().unwrap(),
            };
            std::fs::create_dir_all(s.drm()).unwrap();
            std::fs::create_dir_all(s.pci()).unwrap();
            s
        }
        fn drm(&self) -> PathBuf {
            self.dir.path().join("class/drm")
        }
        fn pci(&self) -> PathBuf {
            self.dir.path().join("bus/pci/devices")
        }
        fn put(&self, rel: &str, v: &str) {
            let p = self.dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, v).unwrap();
        }
        /// A PCI function at `devices/<path>`, listed on the PCI bus.
        fn pci_device(&self, path: &str, class: &str, vendor: &str, device: &str) {
            self.put(&format!("devices/{path}/class"), class);
            self.put(&format!("devices/{path}/vendor"), vendor);
            self.put(&format!("devices/{path}/device"), device);
            let slot = path.rsplit('/').next().unwrap();
            std::os::unix::fs::symlink(
                self.dir.path().join("devices").join(path),
                self.pci().join(slot),
            )
            .unwrap();
        }
        /// DRM `card` whose device is `devices/<path>`.
        fn card(&self, card: &str, path: &str, uevent: &str) {
            self.put(&format!("devices/{path}/uevent"), uevent);
            std::fs::create_dir_all(self.drm().join(card)).unwrap();
            std::os::unix::fs::symlink(
                self.dir.path().join("devices").join(path),
                self.drm().join(card).join("device"),
            )
            .unwrap();
        }
        fn connector(&self, name: &str, status: &str, modes: &str) {
            self.put(&format!("class/drm/{name}/status"), status);
            self.put(&format!("class/drm/{name}/modes"), modes);
        }
        fn detect(&self) -> Vec<Gpu> {
            detect_gpus_in(&self.drm(), &self.pci())
        }
    }

    #[test]
    fn a_virtio_gpu_is_one_gpu_with_its_pci_address() {
        // QEMU: the DRM card's device is `virtio0`, a child of the PCI
        // function, and its uevent names no PCI slot.
        let fs = Sysfs::new();
        fs.pci_device("pci0000:00/0000:00:02.0", "0x030000", "0x1af4", "0x1050");
        fs.card(
            "card0",
            "pci0000:00/0000:00:02.0/virtio0",
            "DRIVER=virtio_gpu\nMODALIAS=virtio:d00000010v00001AF4\n",
        );
        fs.connector("card0-Virtual-1", "connected", "1280x800\n");
        let gpus = fs.detect();
        assert_eq!(gpus.len(), 1, "{gpus:#?}");
        assert_eq!(gpus[0].pci_slot, "0000:00:02.0");
        assert_eq!(gpus[0].pci_id, "1AF4:1050");
        assert_eq!(gpus[0].connected_outputs, ["Virtual-1"]);
        assert_eq!(pick_render_gpu(&gpus), Some(0));
    }

    #[test]
    fn a_usb_display_adapter_does_not_borrow_its_controllers_address() {
        let fs = Sysfs::new();
        fs.pci_device("pci0000:00/0000:00:14.0", "0x0c0330", "0x8086", "0xa36d");
        fs.card(
            "card0",
            "pci0000:00/0000:00:14.0/usb3/3-1/3-1:1.0",
            "DRIVER=udl\n",
        );
        let gpus = fs.detect();
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].pci_slot, "");
    }

    #[test]
    fn cards_are_listed_by_number_and_the_apu_is_told_by_its_rail() {
        let fs = Sysfs::new();
        for (card, slot, vendor_attr, rail) in [
            ("card10", "0000:03:00.0", true, false),
            ("card2", "0000:0a:00.0", false, true),
        ] {
            let path = format!("pci0000:00/0000:00:01.1/{slot}");
            fs.pci_device(&path, "0x030000", "0x1002", "0x7590");
            fs.card(
                card,
                &path,
                &format!("DRIVER=amdgpu\nPCI_ID=1002:7590\nPCI_SLOT_NAME={slot}\n"),
            );
            fs.put(&format!("devices/{path}/hwmon/hwmon1/temp1_input"), "40000");
            fs.put(&format!("devices/{path}/hwmon/hwmon1/in0_label"), "vddgfx");
            if rail {
                fs.put(&format!("devices/{path}/hwmon/hwmon1/in1_label"), "vddnb");
            }
            if vendor_attr {
                fs.put(&format!("devices/{path}/mem_info_vram_vendor"), "samsung");
            }
        }
        let gpus = fs.detect();
        let cards: Vec<&str> = gpus.iter().map(|g| g.card.as_str()).collect();
        assert_eq!(cards, ["card2", "card10"]);
        assert!(!gpus[0].discrete, "the APU has the northbridge rail");
        assert!(gpus[1].discrete);
    }

    #[test]
    fn a_firmware_framebuffer_and_a_device_on_vfio_are_listed_but_not_chosen() {
        let fs = Sysfs::new();
        fs.card(
            "card0",
            "platform/simple-framebuffer.0",
            "DRIVER=simple-framebuffer\n",
        );
        fs.connector("card0-Unknown-1", "connected", "1920x1080\n");
        fs.pci_device(
            "pci0000:00/0000:00:01.0/0000:01:00.0",
            "0x030000",
            "0x10de",
            "0x2684",
        );
        std::os::unix::fs::symlink(
            fs.dir.path().join("bus/pci/drivers/vfio-pci"),
            fs.dir
                .path()
                .join("devices/pci0000:00/0000:00:01.0/0000:01:00.0/driver"),
        )
        .unwrap();
        let gpus = fs.detect();
        assert_eq!(gpus.len(), 2, "both are named in the reports");
        assert_eq!(gpus[1].driver, "vfio-pci");
        assert_eq!(pick_render_gpu(&gpus), None);
        let displays = detect_displays_in(&fs.drm());
        assert_eq!(displays[0].max_mode, Some((1920, 1080)));
        assert_eq!(displays[0].vrr_capable, None, "unknown, not \"no\"");
    }

    #[test]
    fn intel_hwp_takes_its_energy_preference_from_the_power_profile() {
        let cpu = |driver: &str, status: Option<&str>, epp: Option<&str>| Cpu {
            vendor: CpuVendor::Intel,
            model: String::new(),
            physical_cores: 4,
            logical_cpus: 8,
            smt: true,
            hybrid: false,
            scaling_driver: Some(driver.into()),
            available_governors: vec!["performance".into(), "powersave".into()],
            current_governor: Some("powersave".into()),
            available_epp: Vec::new(),
            current_epp: epp.map(Into::into),
            amd_pstate_status: status.map(Into::into),
            vcache: None,
        };
        assert!(
            cpu("intel_pstate", None, Some("balance_performance")).epp_driven_by_power_profile()
        );
        // Active without HWP, and passive mode, have no EPP to drive.
        assert!(!cpu("intel_pstate", None, None).epp_driven_by_power_profile());
        assert!(!cpu("intel_cpufreq", None, None).epp_driven_by_power_profile());
        assert!(
            cpu("amd-pstate-epp", Some("active"), Some("performance"))
                .epp_driven_by_power_profile()
        );
        assert!(!cpu("amd-pstate", Some("passive"), None).epp_driven_by_power_profile());
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
