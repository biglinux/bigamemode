# AMD neural rendering under Proton — proof of concept

**Result: not currently compatible with Proton on this machine.** BiGame-mode
detects the component, states what is missing, and links the official
page. Nothing was downloaded, installed, modified or reverse engineered.

## What the component is

DLSS-NR-on-AMD (danielblnc) runs NVIDIA's neural-rendering model on RDNA 3
and 4 cards on Windows: a proxy DLL beside the game (one of `version.dll`,
`winmm.dll`, `dbghelp.dll`, `wininet.dll`, `winhttp.dll`, `dxgi.dll`),
`dlssnr_on_amd.ini`, `dlssnr_on_amd_weights.bin`, a log, AMD's Windows HIP
runtime (`amdhip64_7.dll`, from the Adrenalin driver) for its kernels, and
the user's own `nvngx_dlssnr.dll` (310.8) as the model. It sits on the
game's FSR path (FidelityFX API) and needs DirectX 12. Its license allows
personal use only and forbids redistribution, bundling, modification and
reverse engineering (`AI_GRAPHICS_LICENSE_AUDIT.md`).

## What was possible here

| Requirement | On the reference desktop | Read by |
|---|---|---|
| RDNA 3 / 4 | RX 9060 XT, RDNA 4 — met | `backend::check` |
| 64-bit Windows game, DirectX 12, FidelityFX API | Cyberpunk 2077 — met; Shadow of the Tomb Raider — no FSR path | `backend::check` |
| AMD HIP runtime in the prefix | **missing**: Proton ships none, the Adrenalin driver does not install under Wine | `ProtonInfo::hip_runtime` |
| the neural-rendering model | **missing**: the user did not provide one; BiGame-mode never says where to get it | `external::Installed::model` |
| the component itself | not present; no legitimately provided binary was given for this session | `external::installed()` |

With two hard requirements absent, no run under Proton was attempted. A
POC would need the user's own copy of the component and the model, and a
HIP runtime that Wine can load, which no Linux driver package provides in
2026-09; the question of whether HIP kernels run at all through Wine on
RADV is open and not one BiGame-mode can answer without those files.

## What BiGame-mode does

- `external::Status`: Unavailable (with the list of missing requirements),
  Not installed, Installed, Loaded (proxy mapped in the running game),
  Active (its log's banner since the process started), Failed (its log's
  errors), Blocked (anti-cheat).
- The page's Neural rendering group: Experimental badge, the state, the
  missing items, "Open official page", "Detect again". No Install, no
  Remove, no download.
- Conflicts: with OptiScaler (one DLL slot, two hooks on the same path):
  the plan treats them as exclusive; with anti-cheat: blocked; with the
  game's own FSR: experimental, the only documented combination.
- The support report carries `neural.json` and the component's log.

## What would change the verdict

A HIP runtime the component accepts under Wine, the user's own files
beside a DX12 game with FSR, and then the same evidence BiGame-mode asks of
every backend: the proxy mapped, the banner in its log, and an A/B
benchmark against the game's own FSR. Until then the page says
"Experimental" and "unavailable", never "compatible".
