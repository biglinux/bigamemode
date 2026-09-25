# Hybrid Intel + NVIDIA laptop audit

What BiGame-mode does on a laptop whose panel is driven by the integrated GPU
and whose games are meant to run on a discrete NVIDIA GPU — checked on real
hardware, not assumed. Every number below was read on the machine.

## The machine

| | |
|---|---|
| Laptop | Dell, chassis "laptop", **no battery** (`ACPI: battery: Slot [BAT0] (battery absent)`); the two `hidpp_battery_*` supplies are a Logitech keyboard and mouse (`scope=Device`) |
| CPU | Intel Core i7-7700HQ, 4 cores / 8 threads, `intel_cpufreq` (intel_pstate **passive**), governors conservative/ondemand/userspace/powersave/performance/schedutil, EPP `balance_performance`, 0.8–3.8 GHz, turbo on |
| Integrated GPU | Intel HD Graphics 630 `8086:591b`, `i915`, Mesa 26.2.2 — **card1 / renderD128**, `boot_vga=1`, drives the eDP panel |
| Discrete GPU | NVIDIA GeForce GTX 1050 Ti Mobile `10de:1c8c` (GP107M, Pascal, **no DLSS**), 4 GiB, driver 580.178.04 (`linux612-nvidia-580xx`) — **card0 / renderD129**, `boot_vga=0`, drives no internal output (its HDMI port is external); BAR1 256 MiB (no Resizable BAR); board power limit not exposed (`power.limit [N/A]`) |
| Kernel / session | 6.12.108-1-MANJARO, KDE Plasma on Wayland |
| Game stack | Proton Experimental 11.0 (DXVK v3.1.1-21, VKD3D-Proton 1.1-5609), Steam Linux Runtime 4 |
| Tools | falcond 2.0.2, scx-scheds / scx-tools 1.1.3, power-profiles-daemon 0.30 + power-profiles-daemon-biglinux, Gamescope 3.16.28, MangoHud 0.8.4; `prime-run`, `switcherooctl` (switcheroo-control active) installed |

Vulkan enumerates the Intel GPU first, the GTX second, llvmpipe third.

## How rendering reaches each GPU

| Program | Without anything | Correct way to the GTX | Wrong way |
|---|---|---|---|
| OpenGL (native game) | Intel (`Mesa Intel HD 630`) | `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia` → NVIDIA's own OpenGL 4.6 | `DRI_PRIME=1` → **zink** (OpenGL translated onto NVIDIA's Vulkan) |
| Vulkan (native game) | Intel is listed first; games that pick "the discrete one" pick the GTX | `__VK_LAYER_NV_optimus=NVIDIA_only` lists the GTX first | — |
| DXVK / VKD3D-Proton | the GTX (they prefer a discrete device) | nothing needed | — |

## What BiGame-mode got wrong, and what it does now

### The GPU a game really uses

- **Before.** The Home page never showed it. Details read GPU clock from
  `/sys/class/drm/card1/device/pp_dpm_sclk` — `card1` is the Intel GPU here and
  the file is amdgpu-only — and GPU temperature from the first
  `/sys/class/hwmon/hwmon*/temp1_input` on the machine, whatever sensor that was.
  There was no NVIDIA telemetry at all.
- **Found while checking the detection itself.** `vkcube --gpu_number 0`
  (rendering on Intel) holds `/dev/nvidia0` seven times and the GTX's render
  node, only because the Vulkan loader enumerated it. The rule "any NVIDIA node
  wins" reported it as rendering on NVIDIA.
- **Now.** `running::render_card` removes an NVIDIA card the process only
  enumerated, using the driver's own list of processes holding a graphics
  context (NVML). Shadow of the Tomb Raider's pid is in that list (the Steam
  container does not hide it); `vkcube` on Intel is not. Details names each card
  and its role ("Renders Shadow of the Tomb Raider", "Games start here",
  "Available", asleep), and the Home game card says which GPU the game is on.
- **Telemetry.** `gpu_telemetry` reads each driver its own way: amdgpu hwmon,
  the i915/xe actual clock (all sysfs has per GPU), and NVML loaded at run time
  for the NVIDIA driver. It matches `nvidia-smi` (52 °C, 139 MHz, P8 idle; in the
  game 100 %, 1417 MHz, 67 °C, P0) and reports why the clock is held down —
  in the game the GTX was **at its power limit**. A runtime-suspended dGPU is
  reported asleep and never queried, so the panel cannot keep it awake.

### PRIME render offload

- **Before.** Nothing detected or set any offload variable.
- **Now.** `hardware::offload_for` decides whether games need offload (the
  games' GPU drives no output while another does) and by which switch (NVIDIA's
  variables for the proprietary driver, `DRI_PRIME=pci-…` for Mesa). The
  launcher sets them for games BiGame-mode starts, and Gamescope composites on
  the games' GPU (`--prefer-vk-device 10de:1c8c`) when the installed version
  has the option. Proof: `glxinfo -B` through a BiGame-mode launch plan reports
  `NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2`, OpenGL 4.6.0 NVIDIA 580.178.04, where
  it reports the Intel GPU without it; Gamescope logs
  `selecting physical device 'NVIDIA GeForce GTX 1050 Ti'`.
- **Limit.** A game the Steam client starts runs in Steam's process tree;
  BiGame-mode cannot set its environment. Proton games do not need it. A native
  OpenGL game started by Steam needs `prime-run %command%` in its launch
  options — Diagnostics now says so (the Hybrid graphics check).

### Driver health

- No reset or fault from the driver itself in this boot's kernel log apart
  from two **Xid** errors, both raised by `SOTTR.exe` while OptiScaler's frame
  generation was installed in it:
  - Xid 69 (graphics class error) at 11:17;
  - Xid 31 (MMU fault, copy engine CE0) at 13:05, in the first play session
    after that configuration was left in place; the game closed a minute later.
- OptiScaler's frame generation was then removed from the game (OptiScaler
  kept as FSR 3.1 upscaler only); no Xid since. See
  [TOMB_RAIDER_BENCHMARK.md](TOMB_RAIDER_BENCHMARK.md).
- The driver was not changed: nothing found pointed at it rather than at the
  frame generation injected into the game.

## Not validated here

- Battery behaviour: there is no battery.
- RTX-only paths (DLSS, NVIDIA frame generation): the GTX 1050 Ti has neither.
- X11: the session is Wayland.
- AMD or Intel discrete GPUs (`DRI_PRIME` path): covered by unit tests only.
