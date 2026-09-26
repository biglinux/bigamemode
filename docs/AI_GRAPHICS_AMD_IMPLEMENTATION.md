# AI Graphics on AMD — what was built and what was verified

Reference desktop: Ryzen 7 5700G with its Vega iGPU (idle), Radeon RX 9060 XT
(RDNA 4, Navi 44, 16 GB), Mesa 26.2.2 (RADV exposes `VK_EXT_shader_float8`,
so FSR 4's FP8 model runs), kernel 7.2.7, Proton Experimental 11.0-100, KDE
Plasma Wayland, 3440×1440. Hardware read fresh at every analysis, never from
a saved profile.

## Requirements for FSR 4, as read in 2026-09

| Requirement | Source | How BiGame-mode reads it |
|---|---|---|
| RDNA 4 for the FP8 model; RDNA 3 runs the INT8 build, slower | AMD's FSR 4 release notes; OptiScaler's `Fsr4Update` documentation | `report::family()` from the PCI database name → `Family::Rdna(n)`; `GpuInfo::fsr4` |
| the game exposes FSR 3.1 through the FidelityFX API (`amd_fidelityfx_dx12.dll`) | AMD: FSR 4 is an in-place upgrade of the FSR 3.1 API path | `scan::ComponentKind::FfxApi`, `Native.ffx_api` |
| a provider: Proton's `contrib/amdxcffx64.dll`, copied into every prefix's `system32` | Proton Experimental's `proton` script | `ProtonInfo::fsr4_provider` (the prefix's `system32`) |
| the game must run with `FSR4_UPGRADE=1` (Valve) or `PROTON_FSR4_UPGRADE=1` (GE-Proton); the switch is read by Wine's `amdxc64.dll` with `getenv` | the DLL's strings; verified: without it the provider never mapped, with it Wine's `amdxc` channel logged `Replaced FSR3 with FSR4!` | `fsr4_upgrade.rs`: one launch option, in front, written with Steam closed, read back; `in_environment(pid)` at run time |
| DirectX 12 (VKD3D-Proton); Vulkan games take the FidelityFX Vulkan DLL, untested here | OptiScaler and Proton docs | `backend::check` names the API |

The variable's other spellings and driver flags seen in older guides
(`WINEDLLOVERRIDES=amdxcffx64=n`, `DXIL_SPIRV_CONFIG`, a copied
`amdxcffx64.dll`) were not needed on Proton Experimental 11.0 and are not
written. The plan explains the single variable once instead of sprinkling
it.

## The two AMD paths

| | Game ships the FidelityFX API (FSR 3.1+) | Game ships only DLSS/XeSS or an older FSR |
|---|---|---|
| Example | Cyberpunk 2077 2.3 | Shadow of the Tomb Raider (XeSS 1.x, no FSR) |
| Recommended | **the game's own FSR**, FSR 4 through Proton; one launch option; no files | **OptiScaler 0.9.4** as `dxgi.dll`, its input the game's XeSS (or DLSS with fakenvapi), output FSR, `Fsr4Update=true` |
| Verified | RX 9060 XT: provider mapped, replacement logged, same frame rate as FSR 3.1 (−0.6 %, no difference) | RX 9060 XT: **+10.1 %** over native TAA and +4.9 % over the game's XeSS (`2026-09-24-sottr-ai-graphics`) |
| What the UI says | "FSR 4 expected through Proton" until the game runs; then Home shows "FSR 4 (the game's own, Proton's provider loaded)" | "FSR" — FSR 4 is never claimed from the log; the overlay is the proof |

Both were driven through BiGame-mode's own code: `graphics_native
Cyberpunk2077.exe fsr4 on|off` wrote and removed the launch option in both
Steam accounts (backups kept, read back); `graphics_apply`/Restore placed
and removed OptiScaler in Cyberpunk, with `amd_fidelityfx_dx12.dll` backed
up and the original hash restored.

## Centralized capabilities

Nothing outside `backend.rs`, `report::family()` and `rules.rs` decides by
vendor. The plan asks `backend::check(OptiScaler, &report)` and reads the
family; the page shows each `Missing` as a row. Fresh detection on this
desktop:

```text
Radeon Vega (Cezanne) · AMD (before RDNA)
Radeon RX 9060 XT (games render here) · RDNA 4 · FSR 4 (FP8)
native               upscaling yes · neural no  · frame generation yes · managed no  · VerifiedHere
optiscaler           upscaling yes · neural no  · frame generation yes · managed yes · VerifiedHere
amd_neural_external  upscaling no  · neural yes · frame generation no  · managed no  · Experimental
```

## Hybrid, NVIDIA and Intel unchanged

The GPU a game renders on is still the one DRM fdinfo shows work on; with
two GPUs the plan says which card it is for until the game runs. The NVIDIA
paths (native DLSS on RTX kept; OptiScaler with `[DLSS] Enabled=false` on a
GTX) and Intel Arc paths are untouched; their tests still pass, and the
external backend is `Unavailable` on them with the vendor named.

## Measurements on this machine

| Game | Arms | Result | Session |
|---|---|---|---|
| Shadow of the Tomb Raider, 3440×1440 High | TAA → XeSS Quality → OptiScaler FSR from XeSS | 89.8 → 94.2 → **98.8 fps** | `2026-09-24-sottr-ai-graphics` |
| Cyberpunk 2077, RT Ultra, 3440×1440 | the game's FSR 3.1 → FSR 4 through Proton → OptiScaler FSR from XeSS | 38.4 → 38.2 (no difference) → **36.0 (−6.4 %)** | `2026-09-26-cyberpunk-rx9060xt-native-vs-optiscaler` |

Both are in `graphics-outcomes.json` as rendered frames and the planner
reads them: OptiScaler is Recommended for the first game and the game's own
FSR for the second.
