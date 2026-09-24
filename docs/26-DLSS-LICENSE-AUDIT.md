# 26 — AI Graphics license audit

What BiGame-mode may ship, may fetch for the user, must leave to the user, or
must never touch. Checked on 2026-09-24 against each project's own license
files and terms; "publicly downloadable" was never read as "redistributable".
Development documentation; the application does not read it.

**BiGame-mode ships no third-party graphics binary.** Everything it places in
a game is fetched, at the user's request, from the component's own official
release, checked against a known SHA-256, and cached once per machine.

## Components

| Component | License / terms | Source checked | Decision |
|---|---|---|---|
| **OptiScaler** | GPL-3.0 | `optiscaler/OptiScaler` LICENSE, release notes ("freely downloadable from GitHub/Nexus") | **Fetched on request** from the project's GitHub release, pinned version + SHA-256; never bundled; the release's own `Licenses/` folder stays in the cache beside it |
| fakenvapi (in the OptiScaler release) | MIT | release `Licenses/`, upstream repo | placed only when DLSS inputs need spoofing on AMD/Intel |
| dlssg-to-fsr3 "Nukem" (in the release) | GPL-3.0 | upstream repo | not placed (DLSS-G input is not offered in this pass) |
| **AMD FidelityFX / FSR** DLLs (in the release) | FidelityFX SDK v2 license: unmodified binaries may be redistributed with the notice; no reverse engineering (MIT for v1) | release `Licenses/FidelityFX_v2_LICENSE.md` | placed from the OptiScaler release when FSR is the output |
| **AMD `amdxcffx64.dll`** (FSR 4 driver runtime) | not part of the SDK; no redistribution grant | AMD SDK license, Proton `contrib` | **never placed**; Proton Experimental provides it in the prefix itself |
| **Intel XeSS** DLLs (in the release) | Intel XeSS license: unmodified binaries redistributable with the license text | release `Licenses/XeSS_LICENSE.txt` | placed only when XeSS is the *output*; the game's own copy is used for XeSS *input* |
| **Microsoft Agility SDK** (`D3D12_Optiscaler/D3D12Core.dll` in the release) | DirectX license: "solely for use on Windows" | release `Licenses/DirectX_LICENSE.txt` | **never placed**: the license excludes this use, and VKD3D-Proton implements D3D12 |
| **NVIDIA DLSS** (`nvngx_dlss*.dll`), **Streamline** (`sl.*.dll`) | NVIDIA RTX SDK license: distribution only "as incorporated into" an application with "material additional functionality"; not "as a stand-alone product" | `NVIDIA/DLSS` and `NVIDIA-RTX/Streamline` license files | **never fetched, placed or replaced**; only the game's own copies are used, where they are |
| **ReShade** | source BSD-3-Clause; reshade.me: "Do NOT share the binaries or shader files. Link users to this website instead." The add-on build "is intended for singleplayer games only" | reshade.me, `crosire/reshade` LICENSE | **not installed** by BiGame-mode in this pass: the binary terms ask for users to be sent to the site, not for tools to fetch on their behalf. Detected when present (as a `dxgi.dll`/`d3d12.dll` owner) |
| **RenoDX** mods | MIT | `clshortfuse/renodx` LICENSE, `renodx.com/games-index.json` | **detected and reported** ("available for this game"), not installed: needs ReShade's add-on build (above) and Microsoft's `d3dcompiler_47.dll` under Proton, and upstream does not support Linux |
| Microsoft `d3dcompiler_47.dll` | Microsoft redistribution terms not confirmed | — | never fetched |
| **lsfg-vk** | v2: CC BY-NC-4.0 (v1.0.0 was MIT); needs a Lossless Scaling purchase | lsfg-vk.dev | unchanged: BiGame-mode configures it when installed, never installs it |
| **DLSS 5 "neural rendering"** (`nvngx_dlssnr.dll`, `renodx-dlss5.addon64`) | leaked, closed, unlicensed | both reference projects say so; `RankFTW/rhi-repo` has no license | **never** — not bundled, not fetched, not linked, not suggested |
| DLSS5-Feeder | MIT code, but requires the leaked pieces | its repository | not used |
| LumeniteFX | AGNYA: source-available, no rehosting; listing it needs the author's permission | its repository | not used |
| Community dataset: AreWeAntiCheatYet `games.json` | MIT (logos from SteamGridDB not covered) | `AreWeAntiCheatYet` repo | not used in this pass; markers on disk are the evidence |
| SteamDB file-detection rules (anti-cheat markers) | MIT | `SteamDatabase/FileDetectionRuleSets` | marker names used as facts in the scanner |

## The reference projects' own licenses

- **DLSS5oneclick** is MIT: ideas and approach could be reused with notice.
  None of its code was copied; the implementation here is written for this
  project.
- **DLSS-5-MANAGER** has no license ("All rights reserved" in
  `Copyright.txt`) despite "Open Source" in its README: nothing from it may be
  reused, and nothing was. It was read for its approach only.

## What the application does to stay on the right side

- The only network access AI Graphics makes is the user-initiated download of
  a pinned OptiScaler release (and, when asked, one GitHub API call for the
  latest stable version). OptiScaler's own in-game update check is switched
  off in the configuration BiGame-mode writes.
- The license of the fetched component is recorded in the cache
  (`release.json`) and in each game's manifest source.
- Files placed are exactly those the chosen configuration needs; files the
  terms exclude (Agility SDK) or that are not redistributable (NVIDIA, AMD's
  driver DLL) are never among them.
