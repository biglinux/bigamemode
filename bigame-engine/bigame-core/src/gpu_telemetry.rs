//! What one GPU is doing right now: load, clock, temperature, power, memory.
//!
//! Each driver exposes this differently, and a reader that assumes one of them
//! reports another card's sensor, or nothing, as the GPU:
//!
//! * **amdgpu** — the card's own hwmon (`freq1_input`, `temp1_input`,
//!   `power1_average`), `gpu_busy_percent` and `mem_info_vram_*`.
//! * **i915 / xe** — only the actual GPU clock is in sysfs (`rps_act_freq_mhz`,
//!   `act_freq`); load, temperature and power are not exposed per GPU.
//! * **nvidia** (proprietary) — nothing useful in sysfs. The driver ships NVML
//!   (`libnvidia-ml.so.1`), loaded here at run time, so machines without it
//!   need nothing and nothing is linked. It answers in microseconds, where
//!   running `nvidia-smi` for every sample costs a process each time.
//!
//! A discrete GPU in runtime suspend (`power/runtime_status` = `suspended`) is
//! not queried: asking NVML wakes it, and a laptop whose dGPU is kept awake by
//! a telemetry panel loses battery for nothing. It is reported as asleep, which
//! is the answer.

use std::path::{Path, PathBuf};

use crate::hardware::{Gpu, GpuVendor};

/// One reading of one GPU. Every field is optional: drivers expose different
/// subsets, and an absent value is shown as absent, never as zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuSample {
    /// DRM card, e.g. `card0`.
    pub card: String,
    /// The device is in runtime suspend, so it was not queried.
    pub asleep: bool,
    /// Utilisation, percent.
    pub busy_pct: Option<u32>,
    /// Graphics clock, MHz.
    pub clock_mhz: Option<u32>,
    /// Temperature, °C.
    pub temp_c: Option<f64>,
    /// Board power, W.
    pub power_w: Option<f64>,
    /// Video memory in use, MiB.
    pub vram_used_mib: Option<u64>,
    /// Video memory total, MiB.
    pub vram_total_mib: Option<u64>,
    /// NVIDIA performance state (0 = P0, fastest).
    pub pstate: Option<u32>,
    /// Why the clock is held below its maximum right now (NVIDIA): the
    /// firmware's own reasons, e.g. the power cap.
    pub limited_by: Vec<ClockLimit>,
}

/// A reason the driver gives for holding the graphics clock down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockLimit {
    /// The board's power limit.
    PowerCap,
    /// Temperature, by the driver.
    Thermal,
    /// Temperature or power brake, by the hardware.
    HardwareSlowdown,
}

/// Which of `gpus` to report for games: the card the running game has open,
/// when that is known, otherwise the one games are expected to start on.
///
/// On a hybrid laptop these differ from "the most powerful card": an OpenGL
/// game without PRIME offload renders on the integrated GPU, and reporting
/// the idle discrete one beside it would describe a GPU the game is not using.
#[must_use]
pub fn games_gpu(gpus: &[Gpu], running_card: Option<&str>) -> Option<usize> {
    running_card
        .and_then(|c| gpus.iter().position(|g| g.card == c))
        .or_else(|| crate::hardware::pick_render_gpu(gpus))
}

/// Read what `gpu` is doing now.
#[must_use]
pub fn sample(gpu: &Gpu) -> GpuSample {
    let mut s = GpuSample {
        card: gpu.card.clone(),
        ..GpuSample::default()
    };
    if runtime_suspended(&gpu.device_path) {
        s.asleep = true;
        return s;
    }
    match gpu.driver.as_str() {
        "amdgpu" => read_amdgpu(&gpu.device_path, gpu.hwmon.as_deref(), &mut s),
        "i915" | "xe" => read_intel(&gpu.device_path, &mut s),
        "nvidia" => {
            if let Some(bus) = pci_bus_id(&gpu.device_path) {
                nvml::read(&bus, &mut s);
            }
        }
        _ if gpu.vendor == GpuVendor::Amd => {
            read_amdgpu(&gpu.device_path, gpu.hwmon.as_deref(), &mut s);
        }
        _ => {}
    }
    s
}

/// Whether the PCI device behind a DRM card is runtime-suspended.
fn runtime_suspended(device: &Path) -> bool {
    std::fs::read_to_string(device.join("power/runtime_status"))
        .is_ok_and(|v| v.trim() == "suspended")
}

/// `0000:01:00.0` from `/sys/class/drm/cardN/device`.
fn pci_bus_id(device: &Path) -> Option<String> {
    let real = std::fs::canonicalize(device).ok()?;
    let name = real.file_name()?.to_str()?.to_owned();
    // domain:bus:device.function
    (name.len() == 12 && name.as_bytes()[4] == b':' && name.as_bytes()[7] == b':').then_some(name)
}

fn read_u64(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// amdgpu: hwmon plus the device's own attributes.
fn read_amdgpu(device: &Path, hwmon: Option<&Path>, s: &mut GpuSample) {
    if let Some(h) = hwmon {
        s.clock_mhz =
            read_u64(&h.join("freq1_input")).and_then(|hz| u32::try_from(hz / 1_000_000).ok());
        #[allow(clippy::cast_precision_loss)]
        {
            s.temp_c = read_u64(&h.join("temp1_input")).map(|m| m as f64 / 1000.0);
            s.power_w = read_u64(&h.join("power1_average"))
                .or_else(|| read_u64(&h.join("power1_input")))
                .map(|uw| uw as f64 / 1_000_000.0);
        }
    }
    s.busy_pct = read_u64(&device.join("gpu_busy_percent")).and_then(|p| u32::try_from(p).ok());
    s.vram_used_mib = read_u64(&device.join("mem_info_vram_used")).map(|b| b >> 20);
    s.vram_total_mib = read_u64(&device.join("mem_info_vram_total")).map(|b| b >> 20);
}

/// i915 / xe: the actual GPU clock is all sysfs has per GPU.
fn read_intel(device: &Path, s: &mut GpuSample) {
    // `device` is `/sys/class/drm/cardN/device`; the gt attributes hang off
    // the card directory (i915) or the PCI device's tiles (xe).
    let card: PathBuf = device.parent().map(Path::to_path_buf).unwrap_or_default();
    let candidates = [
        card.join("gt/gt0/rps_act_freq_mhz"),
        card.join("gt_act_freq_mhz"),
        device.join("tile0/gt0/freq0/act_freq"),
    ];
    s.clock_mhz = candidates
        .iter()
        .find_map(|p| read_u64(p))
        .and_then(|m| u32::try_from(m).ok());
}

/// The part of NVML this reads, loaded with `dlopen` on first use.
mod nvml {
    use std::ffi::{CString, c_char, c_int, c_uint, c_ulonglong, c_void};
    use std::sync::OnceLock;

    use super::GpuSample;

    type Device = *mut c_void;
    type Ret = c_int;
    const SUCCESS: Ret = 0;

    #[repr(C)]
    #[derive(Default)]
    struct Utilization {
        gpu: c_uint,
        memory: c_uint,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        total: c_ulonglong,
        free: c_ulonglong,
        used: c_ulonglong,
    }

    /// `nvmlMemory_v2_t`: `used` excludes what the driver reserves, as
    /// `nvidia-smi` reports it; v1's `used` includes it.
    #[repr(C)]
    #[derive(Default)]
    struct MemoryV2 {
        version: c_uint,
        total: c_ulonglong,
        reserved: c_ulonglong,
        free: c_ulonglong,
        used: c_ulonglong,
    }

    /// `nvmlProcessInfo_t` (r580; the v3 call uses this layout).
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ProcessInfo {
        pid: c_uint,
        used_gpu_memory: c_ulonglong,
        gpu_instance_id: c_uint,
        compute_instance_id: c_uint,
    }

    struct Api {
        by_pci: unsafe extern "C" fn(*const c_char, *mut Device) -> Ret,
        graphics_processes:
            Option<unsafe extern "C" fn(Device, *mut c_uint, *mut ProcessInfo) -> Ret>,
        temperature: unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> Ret,
        clock: unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> Ret,
        utilization: unsafe extern "C" fn(Device, *mut Utilization) -> Ret,
        power: unsafe extern "C" fn(Device, *mut c_uint) -> Ret,
        memory: unsafe extern "C" fn(Device, *mut Memory) -> Ret,
        memory_v2: Option<unsafe extern "C" fn(Device, *mut MemoryV2) -> Ret>,
        pstate: unsafe extern "C" fn(Device, *mut c_uint) -> Ret,
        reasons: unsafe extern "C" fn(Device, *mut c_ulonglong) -> Ret,
    }

    // The function pointers are process-global symbols of a thread-safe
    // library; sharing them between threads is what NVML documents.
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    static API: OnceLock<Option<Api>> = OnceLock::new();

    fn api() -> Option<&'static Api> {
        API.get_or_init(load).as_ref()
    }

    /// Resolve `name` in `lib`, as a function pointer of type `T`.
    ///
    /// # Safety
    /// `T` must be the function's real signature.
    unsafe fn sym<T: Copy>(lib: *mut c_void, name: &str) -> Option<T> {
        let c = CString::new(name).ok()?;
        // SAFETY: `lib` is a live handle from dlopen, `c` a valid C string.
        let p = unsafe { libc::dlsym(lib, c.as_ptr()) };
        if p.is_null() {
            return None;
        }
        // SAFETY: the caller guarantees `T` is this symbol's function type,
        // and function pointers have the size of a data pointer on Linux.
        Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
    }

    fn load() -> Option<Api> {
        let name = CString::new("libnvidia-ml.so.1").ok()?;
        // SAFETY: dlopen with a valid C string; the handle is never closed, so
        // the symbols below stay valid for the life of the process.
        let lib = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if lib.is_null() {
            return None;
        }
        // SAFETY: each type below is the signature from nvml.h (r580).
        unsafe {
            let init: unsafe extern "C" fn() -> Ret = sym(lib, "nvmlInit_v2")?;
            if init() != SUCCESS {
                return None;
            }
            Some(Api {
                by_pci: sym(lib, "nvmlDeviceGetHandleByPciBusId_v2")?,
                graphics_processes: sym(lib, "nvmlDeviceGetGraphicsRunningProcesses_v3"),
                temperature: sym(lib, "nvmlDeviceGetTemperature")?,
                clock: sym(lib, "nvmlDeviceGetClockInfo")?,
                utilization: sym(lib, "nvmlDeviceGetUtilizationRates")?,
                power: sym(lib, "nvmlDeviceGetPowerUsage")?,
                memory: sym(lib, "nvmlDeviceGetMemoryInfo")?,
                memory_v2: sym(lib, "nvmlDeviceGetMemoryInfo_v2"),
                pstate: sym(lib, "nvmlDeviceGetPerformanceState")?,
                // Renamed in newer drivers; both names are exported by r580.
                reasons: sym(lib, "nvmlDeviceGetCurrentClocksEventReasons")
                    .or_else(|| sym(lib, "nvmlDeviceGetCurrentClocksThrottleReasons"))?,
            })
        }
    }

    const INSUFFICIENT_SIZE: Ret = 7;

    /// Processes holding a graphics context on the GPU at `bus_id`, or
    /// `None` when NVML cannot tell.
    pub(super) fn graphics_pids(bus_id: &str) -> Option<Vec<u32>> {
        let api = api()?;
        let list = api.graphics_processes?;
        let bus = CString::new(bus_id).ok()?;
        let mut dev: Device = std::ptr::null_mut();
        // SAFETY: valid C string and out-pointer.
        if unsafe { (api.by_pci)(bus.as_ptr(), &raw mut dev) } != SUCCESS {
            return None;
        }
        let mut cap: c_uint = 64;
        for _ in 0..3 {
            let mut buf = vec![ProcessInfo::default(); cap as usize];
            let mut count = cap;
            // SAFETY: `buf` holds `count` entries of the type nvml.h declares;
            // NVML writes at most `count` and sets it to the number written or
            // needed.
            let r = unsafe { list(dev, &raw mut count, buf.as_mut_ptr()) };
            if r == SUCCESS {
                buf.truncate(count as usize);
                return Some(buf.iter().map(|p| p.pid).collect());
            }
            if r != INSUFFICIENT_SIZE {
                return None;
            }
            cap = count.max(cap * 2);
        }
        None
    }

    const TEMPERATURE_GPU: c_uint = 0;
    const CLOCK_GRAPHICS: c_uint = 0;
    const REASON_SW_POWER_CAP: c_ulonglong = 0x4;
    const REASON_HW_SLOWDOWN: c_ulonglong = 0x8;
    const REASON_SW_THERMAL: c_ulonglong = 0x20;
    const REASON_HW_THERMAL: c_ulonglong = 0x40;
    const REASON_HW_POWER_BRAKE: c_ulonglong = 0x80;
    /// `nvmlPstates_t` uses 32 for "unknown".
    const PSTATE_UNKNOWN: c_uint = 32;

    #[allow(clippy::many_single_char_names)] // out-values of a C API, read once each
    pub(super) fn read(bus_id: &str, s: &mut GpuSample) {
        let Some(api) = api() else { return };
        let Ok(bus) = CString::new(bus_id) else {
            return;
        };
        let mut dev: Device = std::ptr::null_mut();
        // SAFETY: valid C string and out-pointer; NVML fills `dev` on success.
        if unsafe { (api.by_pci)(bus.as_ptr(), &raw mut dev) } != SUCCESS {
            return;
        }
        // SAFETY (every call below): `dev` is a handle NVML just returned and
        // each out-pointer is a live local of the type nvml.h specifies.
        unsafe {
            let mut v: c_uint = 0;
            if (api.temperature)(dev, TEMPERATURE_GPU, &raw mut v) == SUCCESS {
                s.temp_c = Some(f64::from(v));
            }
            if (api.clock)(dev, CLOCK_GRAPHICS, &raw mut v) == SUCCESS {
                s.clock_mhz = Some(v);
            }
            let mut u = Utilization::default();
            if (api.utilization)(dev, &raw mut u) == SUCCESS {
                s.busy_pct = Some(u.gpu);
            }
            // Laptop GeForce boards commonly answer NOT_SUPPORTED here.
            if (api.power)(dev, &raw mut v) == SUCCESS {
                s.power_w = Some(f64::from(v) / 1000.0);
            }
            let mut m2 = MemoryV2 {
                // NVML_STRUCT_VERSION(Memory, 2)
                #[allow(clippy::cast_possible_truncation)]
                version: (std::mem::size_of::<MemoryV2>() as c_uint) | (2 << 24),
                ..MemoryV2::default()
            };
            let mut m = Memory::default();
            if api
                .memory_v2
                .is_some_and(|f| f(dev, &raw mut m2) == SUCCESS)
            {
                s.vram_used_mib = Some(m2.used >> 20);
                s.vram_total_mib = Some(m2.total >> 20);
            } else if (api.memory)(dev, &raw mut m) == SUCCESS {
                s.vram_used_mib = Some(m.used >> 20);
                s.vram_total_mib = Some(m.total >> 20);
            }
            if (api.pstate)(dev, &raw mut v) == SUCCESS && v != PSTATE_UNKNOWN {
                s.pstate = Some(v);
            }
            let mut r: c_ulonglong = 0;
            if (api.reasons)(dev, &raw mut r) == SUCCESS {
                s.limited_by = super::limits_from_reasons(
                    r & REASON_SW_POWER_CAP != 0,
                    r & (REASON_SW_THERMAL | REASON_HW_THERMAL) != 0,
                    r & (REASON_HW_SLOWDOWN | REASON_HW_POWER_BRAKE) != 0,
                );
            }
        }
    }
}

/// Processes that hold a graphics context on the NVIDIA GPU at PCI address
/// `pci_slot` (`0000:01:00.0`), from the driver itself. `None` when NVML is
/// not available — the caller cannot tell rendering from enumerating then.
///
/// A process that only enumerated Vulkan devices keeps `/dev/nvidia0` open
/// without rendering there; only a context shows it is really in use.
#[must_use]
pub fn nvidia_graphics_pids(pci_slot: &str) -> Option<Vec<u32>> {
    nvml::graphics_pids(pci_slot)
}

fn limits_from_reasons(power: bool, thermal: bool, hardware: bool) -> Vec<ClockLimit> {
    let mut v = Vec::new();
    if power {
        v.push(ClockLimit::PowerCap);
    }
    if thermal {
        v.push(ClockLimit::Thermal);
    }
    if hardware {
        v.push(ClockLimit::HardwareSlowdown);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(dir: &Path, card: &str, driver: &str, vendor: GpuVendor, hwmon: Option<PathBuf>) -> Gpu {
        Gpu {
            card: card.into(),
            device_path: dir.join(card).join("device"),
            vendor,
            pci_id: String::new(),
            pci_slot: String::new(),
            driver: driver.into(),
            hwmon,
            connected_outputs: vec![],
            vram_total_bytes: None,
            discrete: true,
            dpm_level_path: None,
        }
    }

    fn put(path: &Path, v: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, v).unwrap();
    }

    #[test]
    fn amdgpu_reads_its_own_hwmon_and_device_attributes() {
        let d = tempfile::tempdir().unwrap();
        let dev = d.path().join("card1/device");
        let hw = dev.join("hwmon/hwmon3");
        put(&hw.join("freq1_input"), "2640000000\n");
        put(&hw.join("temp1_input"), "61000\n");
        put(&hw.join("power1_average"), "163000000\n");
        put(&dev.join("gpu_busy_percent"), "99\n");
        put(&dev.join("mem_info_vram_used"), &(3u64 << 30).to_string());
        put(&dev.join("mem_info_vram_total"), &(16u64 << 30).to_string());
        let s = sample(&gpu(d.path(), "card1", "amdgpu", GpuVendor::Amd, Some(hw)));
        assert_eq!(s.clock_mhz, Some(2640));
        assert_eq!(s.temp_c, Some(61.0));
        assert_eq!(s.power_w, Some(163.0));
        assert_eq!(s.busy_pct, Some(99));
        assert_eq!(
            (s.vram_used_mib, s.vram_total_mib),
            (Some(3072), Some(16384))
        );
        assert!(!s.asleep);
    }

    #[test]
    fn intel_reports_only_the_actual_clock_and_nothing_it_does_not_have() {
        let d = tempfile::tempdir().unwrap();
        put(&d.path().join("card1/gt/gt0/rps_act_freq_mhz"), "1050\n");
        std::fs::create_dir_all(d.path().join("card1/device")).unwrap();
        let s = sample(&gpu(d.path(), "card1", "i915", GpuVendor::Intel, None));
        assert_eq!(s.clock_mhz, Some(1050));
        assert_eq!((s.busy_pct, s.temp_c, s.power_w), (None, None, None));
    }

    #[test]
    fn a_suspended_dgpu_is_asleep_and_not_queried() {
        let d = tempfile::tempdir().unwrap();
        let dev = d.path().join("card0/device");
        put(&dev.join("power/runtime_status"), "suspended\n");
        // Its sensors would read if it were queried; it must not be.
        let hw = dev.join("hwmon/hwmon1");
        put(&hw.join("temp1_input"), "45000\n");
        let s = sample(&gpu(d.path(), "card0", "amdgpu", GpuVendor::Amd, Some(hw)));
        assert!(s.asleep);
        assert_eq!(s.temp_c, None);
    }

    #[test]
    fn no_sensor_is_borrowed_from_another_device() {
        // A card with no hwmon of its own reports no temperature — never the
        // first temp1_input found elsewhere on the machine.
        let d = tempfile::tempdir().unwrap();
        put(&d.path().join("hwmon0/temp1_input"), "72000\n");
        std::fs::create_dir_all(d.path().join("card0/device")).unwrap();
        let s = sample(&gpu(d.path(), "card0", "nouveau", GpuVendor::Nvidia, None));
        assert_eq!(s.temp_c, None);
    }

    #[test]
    fn the_card_the_game_has_open_is_reported_over_the_expected_one() {
        let d = tempfile::tempdir().unwrap();
        let nvidia = gpu(d.path(), "card0", "nvidia", GpuVendor::Nvidia, None);
        let mut intel = gpu(d.path(), "card1", "i915", GpuVendor::Intel, None);
        intel.discrete = false;
        intel.connected_outputs = vec!["eDP-1".into()];
        let gpus = [nvidia, intel];
        // Nothing running: games are expected on the discrete card.
        assert_eq!(games_gpu(&gpus, None), Some(0));
        // An OpenGL game without offload has the Intel card open.
        assert_eq!(games_gpu(&gpus, Some("card1")), Some(1));
        // A card that is gone falls back to the expectation.
        assert_eq!(games_gpu(&gpus, Some("card7")), Some(0));
    }

    #[test]
    fn nvidia_clock_reasons_map_to_what_holds_the_clock_down() {
        assert_eq!(
            limits_from_reasons(true, false, false),
            [ClockLimit::PowerCap]
        );
        assert_eq!(
            limits_from_reasons(false, true, true),
            [ClockLimit::Thermal, ClockLimit::HardwareSlowdown]
        );
        assert!(limits_from_reasons(false, false, false).is_empty());
    }
}
