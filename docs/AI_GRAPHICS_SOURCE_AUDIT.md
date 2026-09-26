# AI Graphics — what was learned from the reference tools

Two Windows tools that install neural-rendering and upscaling mods were
read for their architecture, feature by feature, and each idea was kept,
adapted to Linux/Proton, or rejected. No code was taken from either;
their licenses (`AI_GRAPHICS_LICENSE_AUDIT.md`) and their reliance on a
leaked NVIDIA build settle that before any technical question.

| | BiGame-mode (before) | DLSS5oneclick (Rust, MIT) | DLSS-5-MANAGER (F#, no OSS license) |
|---|---|---|---|
| Platform | Linux, Proton/Wine, native | Windows | Windows |
| Files it places | OptiScaler release, from GitHub, SHA-256 checked | ReShade, add-ons, a leaked DLSS 5 add-on and model, OptiScaler forks, dgVoodoo | payload shipped with its releases (165 MB model, ReShade, add-ons, an AMD payload) |
| Record of what it placed | JSON manifest with hashes and backups, transaction, rollback, recovery | plain-text sidecar markers and line-list manifests | JSON manifest with backups, in AppData and beside the exe |
| Anti-cheat | folder/file markers, blocked without override | folder/file/exe markers, `IGNORE_ANTICHEAT` env override | not handled |

## Feature by feature

| Feature | Where seen | Decision | What BiGame-mode does now |
|---|---|---|---|
| Discover games from launchers | both (registry, `libraryfolders.vdf`, Epic/GOG manifests) | keep ours | Steam, Lutris, Heroic and the menu, from the disk, no registry (`games.rs`) |
| Pick the real executable: `-Shipping.exe`, exclude crash handlers/redists/launchers, walk subfolders | both | **adapt** | already: running process name first, then the Unreal shipping binary, then the largest non-helper; the helper list covers crash/report/launcher/redist/prereq/webhelper (`scan.rs`) |
| Scoring by folder (`Binaries/Win64`, `bin/x64`), version-resource similarity | MANAGER | reject | the running process settles it; deterministic rules beat a score nobody can read |
| Graphics API from PE imports (d3d12 > d3d11 > vulkan-1 > d3d9) | oneclick | keep ours | our PE reader does the same, then the DLLs beside the exe, then the game list, with a confidence per answer; the running game (VKD3D-Proton / DXVK) is the fact |
| DirectX Agility SDK (`D3D12Core.dll`) as DX12 evidence | oneclick | adapt | `d3d12core.dll` beside the exe counts as DX12 evidence; the Agility SDK copy in OptiScaler's release is never placed (Windows-only, useless under VKD3D) |
| Native DLSS/XeSS/FSR detection with versions | both (DLSS only) | keep ours | DLSS, DLSS-G, DLSS-RR, Streamline, XeSS, XeSS-FG, FSR — plus the FidelityFX API (`amd_fidelityfx_dx12.dll`), which is the FSR 4 upgrade path |
| Modes/routes: OptiScaler, DX12, DX11, Vulkan, Emulator, AMD | MANAGER | **adapt** | three backends with stated capabilities (`backend.rs`): Native, OptiScaler, AMD neural (external). Emulators are out of scope |
| A separate AMD route | MANAGER (`AmdMode`) | adapt | `AmdNeuralExternal`: detected, explained, linked; never placed (license) |
| Automatic selection by hardware (RTX tiers, `isRtx40`) | both | adapt | GPU families (`report::Family`): RDNA 1–4, GTX, RTX 20/30/40/50, Arc, Intel integrated; the planner reasons at that granularity |
| Proxy slot selection (first free of dxgi, winmm, version, …) | MANAGER | keep ours | `dxgi.dll` only, which Proton already loads natively; a taken slot stops the plan instead of moving to another (which of two hooks wins is the user's call) |
| Proxy identity by embedded strings (ReShade, OptiScaler, Special K) | both | keep ours | already by content; DLSS-NR-on-AMD's markers added |
| `--check` / Diagnose / report zip | oneclick | **adapt** | `diagnose.rs`: findings with level, what was found and what to do, on the page and in the report; `graphics_diagnose`, `graphics_capabilities` examples; the report zip gains `diagnose.txt`, `system.txt`, `proton.txt`, `conflicts.txt`, `loaded-modules.txt`, `neural.json` |
| Log parsing for "loaded / feature created / failed" | oneclick | keep ours | OptiScaler's log already; DLSS-NR-on-AMD's log read for its banner and errors |
| Installed / not needed / missing per file; ready / partial / stale | both | keep ours | manifest verification (intact / missing / changed), Repair for missing files |
| Version pinning, `--addon=<tag>`, stale detection | oneclick | keep ours | tested / latest / pinned, offered updates, Go back; the game list gains `bad_optiscaler`, `verified_gpu`, `verified_proton`, `notes` |
| Known-good per game (RenoDX games index) | oneclick (RenoDX only) | adapt | `graphics-games.toml`, carried and user-extendable |
| Never downgrade a runtime (Streamline min version) | MANAGER | reject | the game's own runtimes are never replaced except by OptiScaler's own FidelityFX DLL, backed up |
| Hybrid GPU via Windows `UserGpuPreferences` | oneclick | reject | DRM fdinfo: the card the running game submits work to |
| Registry: GPU list, NGX keys, driver version gate | both | reject | sysfs, PCI database, `/proc` |
| Defender/SmartScreen guidance | oneclick | reject | — |
| dgVoodoo for DX9 | both | reject | DXVK already translates D3D9–11 under Proton |
| 32-bit `host64` helper | both | reject | 32-bit games keep their own options; the page says so |
| Leaked "DLSS 5" add-on and model, NGX caller-path gate defeat, GPU spoof for feature 18, MFG unlocks | both | **reject** | none of it; DLSS is never fetched; the neural model is only ever detected |
| Overlay with live control | MANAGER | reject | OptiScaler's own overlay exists; nothing to add |
| Community route reports | MANAGER | reject | measurements stay local (`outcomes.rs`) |

## What changed in BiGame-mode because of this reading

- **Backends as data** (`graphics/backend.rs`): each backend states GPU
  vendors and AMD generations, APIs, whether it needs a Windows game and the
  FidelityFX API, which of the three jobs it does (upscaling, neural
  rendering, frame generation), whether BiGame-mode manages its files, its
  maturity and its risks. `check()` returns what is missing, in words.
- **Three jobs told apart** in the plan: `backend` (who upscales),
  `frame_generation` (which generator is left on: none, the game's own,
  OptiScaler's, lsfg-vk), and the neural status apart from both.
- **Native FSR 4 through Proton** (`fsr4_upgrade.rs`): the Native
  backend's one action, verified in the running game.
- **Diagnose** and a fuller support report.
- **The external AMD neural backend** (`external.rs`): states read from
  files and the running game, requirements from upstream plus the two only a
  Windows machine meets.
- **GPU families** and Proton facts (`report.rs`): the prefix's FSR 4
  provider, HIP runtime, Windows version and Proton build.
