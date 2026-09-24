# 25 — AI Graphics research: DLSS-5-MANAGER, DLSS5oneclick, Bigamemode

What the two reference projects do, what upstream says, and what that means
for BiGame-mode. Development documentation: nothing in the application reads
it.

Sources: the code of both projects at the commits cloned on 2026-09-24 (file
and line references below), the upstream repositories and documentation of
OptiScaler, ReShade, RenoDX, NVIDIA DLSS/Streamline, Intel XeSS, AMD
FidelityFX, VKD3D-Proton, Proton and Gamescope, and tests on the reference
machine (Ryzen 7 5700G, Radeon RX 9060 XT — RDNA 4, Mesa 26.2.2, Proton
Experimental with Wine 11.0). Where a reference project and upstream
disagree, upstream wins; where upstream and a test here disagree, the test is
recorded.

## The finding that decides the scope

**The "DLSS 5" both projects are built around is a leaked NVIDIA build.**
Both say so themselves: DLSS5oneclick's README and GUI call the files
"leaked" (`src/gui.rs:916,962,2471`); DLSS-5-MANAGER's worker refers to "the
leaked runtimes (dcc0dc24 and the 310.8.SF family)"
(`ScreenNative/screen_worker.cpp:253`). The payload is `nvngx_dlssnr.dll`
(NGX feature 18, "Reserved18" in NVIDIA's public header) driven by a closed
ReShade add-on, `renodx-dlss5.addon64`, hosted on an unlicensed repository
(`RankFTW/rhi-repo`) that also mirrors NVIDIA's DLSS, frame-generation and
Streamline DLLs.

NVIDIA's DLSS 5 is real — announced at GTC on 2026-03-16, shipped on
2026-09-03 in NBA 2K27 on RTX 50 with driver 616.64 — but it is integrated
per game through Streamline or the Unreal plugin; there is no public DLSS 5
SDK, and DLSS SDK 310.9.1 contains no `nvngx_dlssnr.dll`.

So: **nothing from the DLSS 5 path is bundled, downloaded, suggested or
supported.** A game that integrates DLSS 5 itself is detected as a game with
native DLSS, like any other. What the two projects are useful for is the
engineering *around* their payloads.

## The three, side by side

| | DLSS-5-MANAGER | DLSS5oneclick | Bigamemode before this work |
|---|---|---|---|
| Platform | Windows only; F#/.NET 8, Avalonia | Windows; Rust, egui | Linux; Rust, GTK4 |
| License | README says "Open Source", `Copyright.txt` says "All rights reserved", no license file, release builds obfuscated | MIT | GPL-3.0-or-later |
| Payload source | a `mod files/` folder not in the repo, plus an anonymous R2 bucket, no hash check (`Services/CloudAssets.fs:18`) | downloaded at install: reshade.me, NVIDIA/DLSS at the latest tag, several forks' releases; only one archive hash-checked (`src/installer.rs:1970-1983`) | a user-supplied folder, copied as is |
| API detection | heuristic that counts proxy `d3d11.dll`/`d3d12.dll` as evidence and expects a `dxvk.dll` DXVK does not ship | PE import table with D3D9/Agility refinements (`src/game.rs:201-579`) | none |
| DLSS detection | any `nvngx.dll` is "DLSS" (`Services/GameScanner.fs:340`) | file presence | none |
| Proxy handling | first free slot (`Services/ModInstaller.fs:578-625`) | refuses a ReShade `dxgi.dll`, **overwrites any other** (DXVK, Special K) without backup (`src/installer.rs:539-544`) | `dxgi.dll`, `nvngx.dll`, `_nvngx.dll`, `OptiScaler.ini` copied over whatever is there, no backup (`launcher.rs: stage_optiscaler_dlls`) |
| Record of changes | install record in AppData and beside the exe; backup-once tracker (`:306-349`) | marker files beside each placed file, install records with tag and source (`src/game.rs:12-84`) | none |
| Uninstall | deletes a ReShade the user had (`:3040-3049`), `OptiScaler.ini` and dgVoodoo files **by name** (`:518-525`) | deletes shader headers and `lumenite_*` **by name** (`:3114-3141`), and a `dxgi.dll` it overwrote | none |
| Anti-cheat | none — installs into EAC/BattlEye titles | refusal by markers and exe names (`src/game.rs:634-707`), with an override | none |
| Diagnostics | — | log reading with Proton findings (`src/diagnose.rs`), report zip, `--check` dry run | a dashboard flag that is true when any mapped file is named `nvngx.dll` |
| Circumvention | worker named `nvngx.dll` to pass NVIDIA's caller check; `NvAPI_GPU_GetArchInfo` patched to fake Blackwell | — | — |

## What was taken, what was not

**Taken — reimplemented, not copied:**

- A per-game **manifest** of every file placed, with its hash, and a
  **backup of every original** with its hash, verified before anything
  changes. Removal works from the manifest and the hashes, never from names.
  (The idea behind both projects' records; neither removes only by hash.)
- **PE import-table** reading for the API, reading only the table's bytes
  (DLSS5oneclick's approach), plus the file version from the version
  resource.
- **Proxy-slot ownership by content**, with the version resource read first
  (a few hundred bytes) and the whole file only when it says nothing.
- **Section-scoped ini edits** that keep the rest of `OptiScaler.ini`.
- An **anti-cheat gate** by markers and executable names — without an
  override: the account is the user's, and a disabled injection costs less
  than a ban.
- **Pinned known-good versions**, with "latest stable" opt-in and checked
  against the digest GitHub publishes.
- A **dry run** (plan) before any change, and a **support report**.

**Not taken:**

- Everything in the leaked DLSS 5 path, the NVIDIA-restriction circumventions,
  and bundling of any third-party binary.
- Unverified downloads, self-update, name-based cleanup, `nvngx.dll`-means-DLSS,
  and API guesses from proxy-named DLLs.
- Replacing Streamline or NVIDIA DLLs in games.

**Replaced in Bigamemode:** `launcher::stage_optiscaler_dlls` (copies over
anything, no backup, no removal), the dashboard's "OptiScaler active" (true
for any mapped `nvngx.dll`), and the "AFMF" frame-generation backend — it sets
`RADV_PERFTEST=afmf`, an option that does not exist: the installed RADV
(Mesa 26.2.2) contains no `afmf` string at all, while its real `RADV_PERFTEST`
options are all there. AMD Fluid Motion Frames is a Windows driver feature.

## Upstream facts the implementation rests on

| Fact | Source | Here |
|---|---|---|
| OptiScaler is `optiscaler/OptiScaler`, GPL-3.0; latest stable v0.9.4 (2026-07-18), one `.7z` asset, SHA-256 `575cb4df…ef0ad` | GitHub release + API digest | downloaded, hash matched |
| The archive overwrites a game's own `libxess.dll` and `amd_fidelityfx*.dll` if extracted as upstream says; upstream's own uninstall script leaves them | release contents, `setup_linux.sh` | why backup-first |
| OptiScaler hooks whichever `libxess.dll` the game loaded: XeSS *input* needs no replacement XeSS | `dllmain.cpp` (`XeSSProxy::InitXeSS` on the module in memory) | **VERIFIED**: SotTR's own XeSS 1.1 was taken over |
| No `fsr4` ini value: FSR 4 is the `fsr31` backend plus `Fsr4Update` (auto on RDNA 3/4) | `Config.cpp`, `FSR4Upgrade.cpp` | log: `RDNA4: true, Fsr4Update: true` |
| On RDNA 4, `fsr31` is chosen automatically only if Windows 11 is reported or its Agility SDK copy is loaded; Proton reports Windows 10 | `FSR4Upgrade.cpp` | log: `Windows 10 (10.0.19045)` — so `Dx12Upscaler=fsr31` is written explicitly |
| Silent FSR 2.1 fallback if the FFX module fails to load | `FeatureProvider_Dx12.cpp` | checked: `amd_fidelityfx_dx12.dll methods loaded!` |
| **Proton loads a `dxgi.dll` from the game folder with no override** (it sets `dxgi` native for DXVK; the application folder is searched first) | Proton script; the reference projects say an override is needed | **VERIFIED**: `OptiScaler working as dxgi.dll, system dll loaded`, the game-folder DLL mapped in the process |
| Valve's Proton has no `PROTON_FSR4_UPGRADE`/`PROTON_DLSS_UPGRADE`/`PROTON_ENABLE_HDR`/`WINE_FULLSCREEN_FSR`; those are GE-Proton's | Proton and GE-Proton sources | — |
| Proton Experimental puts AMD's FSR 4 runtime `amdxcffx64.dll` in the prefix | Proton `contrib` | log: `amdxcffx64.dll loaded from system path` |
| A Valve developer reports that path renders the FSR 3 model on RDNA 4 (Proton issue #9908) | GitHub issue | **not verified here**: whether the FSR 4 model runs is shown only by OptiScaler's overlay |
| ReShade: source BSD-3; "Do NOT share the binaries… link users to this website"; the add-on build is for single-player games | reshade.me | not installed by BiGame-mode (see [26](26-DLSS-LICENSE-AUDIT.md)) |
| RenoDX (MIT) needs ReShade 6.8+ with add-on support; SotTR and Cyberpunk have stable mods; Linux is not officially supported; under Proton it needs Microsoft's `d3dcompiler_47.dll` | renodx.com `games-index.json`, wiki, issues | detected and reported, not installed |
| lsfg-vk v2 is CC BY-NC-4.0 and needs a Lossless Scaling purchase | lsfg-vk.dev | unchanged; harmony only |

## Risks

- **FSR 4 vs FSR 3.** On this machine the log proves OptiScaler runs AMD's
  upscaler at the game's XeSS render resolution; which model runs is not in
  the log. The UI says "FSR (OptiScaler)" and states FSR 4 only when the
  watermark/overlay is checked — never from the configuration alone.
- **Game updates** can replace or delete placed files; the manifest's hashes
  show it (Repair puts back missing files; changed binaries are reported).
- **Spoofing** (needed for DLSS inputs on AMD/Intel) can push a game onto
  NVIDIA-only code paths; it is used only when the game has no XeSS or FSR
  input to take over, and marked Experimental.
- **OptiScaler is a DLL injection.** "Do not use in multiplayer games" is
  upstream's own rule; the anti-cheat gate enforces it.
