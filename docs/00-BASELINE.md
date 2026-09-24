# 00 — Baseline

Environment captured **before** any change, on the real development machine.
Collected 2026-09-23 from the branch point `7f98d40`.

No passwords, tokens, keys or cookies are recorded here. MAC addresses, public
IPv6 addresses and VPN node identifiers observed during collection were
deliberately omitted.

---

## 1. Reference machine ("BENCH-1")

This is a **desktop** (`hostnamectl chassis` → `desktop`, no `/sys/class/power_supply`
entries), so every laptop/battery code path in this report is marked
`NOT TESTED — hardware unavailable`.

### 1.1 OS / kernel

| Item | Value |
|---|---|
| Distro | BigLinux (`ID=biglinux`, `ID_LIKE=arch`), Manjaro base, rolling |
| Kernel | `7.2.6-x64v3-xanmod1-1`, `PREEMPT_DYNAMIC`, x86-64-v3 build |
| Session | Wayland (`XDG_SESSION_TYPE=wayland`), KDE Plasma |
| Memory | 46 GiB RAM, 70 GiB swap |

### 1.2 CPU

| Item | Value |
|---|---|
| Model | AMD Ryzen 7 5700G with Radeon Graphics (Cezanne, Zen 3 APU) |
| Family / model | 25 / 80, microcode `0xa500012` |
| Topology | 1 socket × 8 cores × 2 threads = 16 logical CPUs, 1 NUMA node |
| Cache | L1d/L1i 256 KiB each, L2 4 MiB, **L3 16 MiB single slice** |
| 3D V-Cache | **Absent.** `/sys/bus/platform/drivers/amd_x3d_vcache/` does not exist |
| Hybrid P/E cores | No — uniform Zen 3 cores |
| Scaling driver | `amd-pstate-epp` |
| `amd_pstate/status` | `active` |
| Governor at rest | `performance` (all cores) |
| Available governors | `performance`, `powersave` **only** |
| EPP at rest | `performance` |
| Boost | enabled; 422 MHz – 4673 MHz |

The restricted governor list matters: on `amd-pstate-epp` there is no
`ondemand`/`schedutil`/`conservative`. Any code offering those governors is
offering something this machine cannot accept.

### 1.3 GPU — dual AMD

Two amdgpu devices are present. This is the single most useful property of this
bench, because it exposes "first card wins" bugs.

| DRM node | PCI | Device | Role | hwmon |
|---|---|---|---|---|
| `card0` | `0a:00.0` | Cezanne iGPU (Radeon Vega, `1002:1638`) | integrated, **not** used for games | `hwmon2` — `temp1_input`, `freq1_input` |
| `card1` | `03:00.0` | **Radeon RX 9060 XT** (Navi 44, `gfx1200`, RDNA 4, `1002:7590`) | renders everything | `hwmon1` — `temp1_input`, `freq1_input`, `power1_average`, `fan1_input` |

- Driver: `amdgpu` for both.
- Mesa `26.2.2`, RADV, ACO. Vulkan instance `1.4.357`, device API `1.4.354`.
- dGPU VRAM 16384 MB; `power_dpm_force_performance_level` = `auto` on **both** cards.
- Only `card1` exposes `power1_average` — power draw telemetry is dGPU-only here.

### 1.4 Displays

All outputs hang off `card1`.

| Output | Mode in use | VRR | HDR |
|---|---|---|---|
| `HDMI-A-1` | 3440×1440 @ **49.95 Hz** | `incapable` | `incapable` |
| `DP-2` | 2560×1080 @ 74.99 Hz | `Never` | — |
| `DP-1` | connected, 3440×1440 available | — | — |

**No VRR and no HDR anywhere on this bench.** Every VRR/HDR claim in this project
is therefore `NOT TESTED — hardware unavailable`. The 49.95 Hz primary mode is
also a good adversarial case for any "cap FPS to refresh rate" logic.

`/sys/class/drm/card*-*/vrr_capable` does not exist on these connectors — VRR
capability had to be read through `kscreen-doctor`. Code that assumes the sysfs
attribute exists will silently mis-detect.

### 1.5 Gaming stack

| Component | State |
|---|---|
| `falcond` | **2.0.2-2**, `active (running)`, enabled, 2.4 MB RSS, binary at `/usr/bin/falcond` |
| Feral GameMode | **not installed** (`gamemoded`/`gamemoderun` absent, no user unit) |
| Gamescope | **3.16.28** |
| MangoHud | 0.8.4-1 |
| power-profiles-daemon | present; `performance` active; `CpuDriver: amd_pstate` |
| Steam | installed, library at `~/.local/share/Steam` |
| lsfg-vk | layer `VK_LAYER_LS_frame_generation` 1.4.313 present |
| vkBasalt | layer present |
| Gamescope WSI layer | present |
| Mesa anti-lag layer | present |

### 1.6 sched-ext / SCX

| Item | Value |
|---|---|
| Kernel support | **Yes** — `/sys/kernel/sched_ext/` exists (`state`, `switch_all`, `nr_rejected`, …) |
| `state` | `disabled` — no BPF scheduler loaded; kernel EEVDF is running |
| Schedulers installed | 16 binaries in `/usr/bin`: `beerland bpfland cake chaos cosmos flash flow forge lavd layered mlfq p2dq pandemonium rustland rusty tickless` |
| `scx_loader` | **not running** — falcond logs `warning(scx_loader): Failed to load initial state: ServiceUnknown` |
| `scxctl` | **not installed** |

This is the decisive fact for the scheduler feature: the kernel and the
scheduler binaries are all present, but the D-Bus service falcond delegates to
(`org.scx.Loader`) does not exist on this system. falcond cannot change the
scheduler here, and neither can BiGame-mode through falcond.

### 1.7 falcond runtime state

`/etc/falcond/config.conf` — this is the file falcond 2.0.2 actually reads
(confirmed with `strings /usr/sbin/falcond`, which contains
`/etc/falcond/config.conf` and `/usr/share/falcond/system.conf`, and **no**
`falcond.conf`):

```
enable_performance_mode = true
scx_sched = none
scx_sched_props = gaming
vcache_mode = none
profile_mode = handheld
poll_interval_ms = 9000
```

`profile_mode = handheld` on a desktop is a pre-existing misconfiguration.

`/tmp/falcond_status`, `-rw-r--r-- root:root`, 262 bytes:

```
FEATURES:
  Performance Mode: Available
CONFIG:
  Profile Mode: handheld
  Global VCache Mode: none
  Global SCX Scheduler: none
AVAILABLE_SCX_SCHEDULERS:
  (None or scx_loader unavailable)
LOADED_PROFILES: 8
ACTIVE_PROFILE: None
QUEUED_PROFILES:
  (None)
```

Profiles on disk: 6 system (`civ7 cs2 cyberpunk2077 factorio ffxiv hades2`),
a `handheld/` subdirectory, and 2 user overrides written by a previous
BiGame-mode run — `Arc Raiders.conf` and `Dead by Daylight.conf`.

Those two user profiles are the first visible symptom of a design bug: falcond
matches `/proc/<pid>/comm`, so a profile keyed on the human title `Arc Raiders`
can never match a running process. See [01-AUDIT.md](01-AUDIT.md).

`bigame-daemon` is **not installed** as a unit on this machine
(`Unit bigame-daemon.service could not be found`), and no
`com.biglinux.BiGameMode.conf` is installed into `/usr/share/dbus-1/system.d/`.
So the currently-shipped D-Bus path is not merely untested here — it cannot run.

### 1.8 Network

| Item | Value |
|---|---|
| Default route | `192.168.0.1` dev `enp7s0` (DHCP, metric 100) |
| Primary link | Ethernet, `1000` Mb/s, MTU 1500, `operstate=up` |
| qdisc on `enp7s0` | **`fq_codel` already** (limit 10240p, target 5 ms, ECN on) |
| DNS | `1.1.1.1`, `1.0.0.1`, then two ISP resolvers |
| `systemd-resolved` | not in use (`resolvectl status` empty) |
| Wi-Fi | none |

The machine also carries **41 other interfaces**: 12 ZeroTier `zt*`, 1
Tailscale, 14 `veth*`, 12 docker/libvirt bridges. Any network feature that
enumerates interfaces instead of following the default route will pick the
wrong one here. `fq_codel` being the existing default also means "we enabled
fq_codel for you" would be a no-op that must not be reported as an improvement.

---

## 2. Project baseline

Branch point `7f98d40`, workspace `bigame-engine/` (Rust 2024, rustc 1.95.0).

| Crate | Purpose | Source lines |
|---|---|---|
| `bigame-core` | system backend | 4 050 |
| `bigame-ui` | GTK4/libadwaita front end | 7 175 |
| `bigame-daemon` | root D-Bus service | 174 |

Toolchain results at the branch point:

| Command | Result |
|---|---|
| `cargo check --workspace` | **pass**, clean |
| `cargo test --workspace` | **pass** — 77 passed, 0 failed |
| `cargo fmt --check` | **fails** — 2 files unformatted (`examples/force_save.rs`, `src/launcher.rs`) |

One caveat on that test run: `test_launch_plan_gamescope_enabled_wraps` and its
siblings only pass here because `LaunchPlan` gates on
`is_turbo_mode_active()`, which falls back to reading the live PowerProfiles
D-Bus state — and this machine happens to sit in `performance`. On a machine in
`balanced` the same tests fail. The suite is not hermetic; see AUDIT `T-01`.

---

## 3. Measured idle baseline

Taken with the desktop idle, no game running, before any change.

| Metric | Value |
|---|---|
| CPU governor / EPP | `performance` / `performance` |
| Power profile (PPD) | `performance` |
| sched_ext state | `disabled` (EEVDF) |
| dGPU `power_dpm_force_performance_level` | `auto` |
| iGPU `power_dpm_force_performance_level` | `auto` |
| falcond active profile | none |

This is the state that Booster Mode must be able to restore **exactly**. Note in
particular that the power profile is already `performance` at rest: the shipped
Booster's "off" path hardcodes `balanced`, so a single toggle of the old Booster
permanently changes this machine's resting state. That is the regression the
snapshot/rollback work exists to fix.
