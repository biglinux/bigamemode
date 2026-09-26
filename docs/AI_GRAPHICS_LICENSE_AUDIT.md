# AI Graphics — license audit

What each third-party project allows, read from its own LICENSE at the
time of writing (2026-09-26), and what BiGame-mode does with it. Nothing
here is legal advice; it is the reading the code follows.

| Project | License | Download automatically? | Redistribute? | Modify? | Bundle in the package? | How BiGame-mode uses it |
|---|---|---|---|---|---|---|
| [OptiScaler](https://github.com/optiscaler/OptiScaler) | GPL-3.0 | yes | yes, with source and notice | yes | yes, under GPLv3 | fetched from the project's own GitHub release when the user presses Apply, SHA-256 checked, placed in the game as a transaction; never bundled |
| [DLSS-NR-on-AMD](https://github.com/danielblnc/DLSS-NR-on-AMD) | proprietary (2026): personal, non-commercial use; **no redistribution, no bundling or embedding in another tool, launcher, installer or package; no modification, patching or reverse engineering** | **no** — its license says "Link to the official release page instead" | no | no | no | detected beside the game (proxy DLL by content, `dlssnr_on_amd.ini`, weights, log), requirements checked, official page linked, state reported. Never fetched, placed or removed |
| NVIDIA DLSS SDK / NGX (`nvngx_dlss*.dll`) | NVIDIA RTX SDKs License: distributable only inside an application with "material additional functionality", for NVIDIA GPUs; **not as a stand-alone item**; no modification | no | no | no | no | detected in games (versions read); never downloaded, copied between games or replaced. The "DLSS 5" neural-rendering model some tools distribute (`nvngx_dlssnr.dll`) is a build NVIDIA has not released as an SDK: BiGame-mode detects it if the user has it and never says where to get it |
| [AMD FidelityFX SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) (`amd_fidelityfx_*.dll`) | dual: binaries under AMD's binary license (redistribute in binary form only, with notice; no reverse engineering); headers and samples MIT | yes | binaries yes, unmodified | binaries no | yes | arrive inside OptiScaler's release, which carries `Licenses/FidelityFX_*`; placed and removed with it |
| [Intel XeSS](https://github.com/intel/xess) (`libxess*.dll`) | Intel Simplified Software License: redistribute unmodified in binary form with notice; no modification, even at run time | yes | yes, unmodified | no | yes | arrives inside OptiScaler's release (`Licenses/XeSS_LICENSE.txt`); the game's own `libxess.dll` is never replaced |
| [fakenvapi](https://github.com/FakeMichau/fakenvapi) | MIT | yes | yes | yes | yes | inside OptiScaler's release; placed only for a DLSS input on a non-NVIDIA GPU |
| [lsfg-vk](https://lsfg-vk.dev) | packaged 1.0.0 (community-extra): GPL-3.0; current upstream source: **CC BY-NC-ND 4.0** (use and reupload allowed, no derivatives, non-commercial) | n/a (a system package) | unmodified only | no | n/a | BiGame-mode writes entries in its configuration file and reads its log; it neither ships nor modifies it. `Lossless.dll` is the user's own, read in place |
| [ReShade](https://github.com/crosire/reshade) | BSD-3-Clause (binaries distributed only from reshade.me) | not done | yes, with notice | yes | not done | detected by content in DLL slots and reported as a conflict; never fetched |
| [RenoDX](https://github.com/clshortfuse/renodx) | MIT | not done | yes | yes | not done | reported as an HDR option that needs ReShade's add-on build; never fetched |
| dgVoodoo 2 | proprietary freeware | not done | n/a | no | no | detected by content (a DLL slot owner); not used: DX9–11 already reach Vulkan through DXVK under Proton |
| Microsoft DirectX Agility SDK (`D3D12_Optiscaler/D3D12Core.dll` in OptiScaler's release) | Microsoft, Windows only | — | — | — | — | never copied into a game: of no use under VKD3D-Proton, and licensed for Windows |
| Proton's FSR 4 provider (`contrib/amdxcffx64.dll`, `amdxc64.dll`) | shipped by Valve inside Proton | — | — | — | — | detected in the game's prefix; BiGame-mode sets the launch option (`FSR4_UPGRADE=1`) that makes Proton use it, and verifies it in the running game. Never copied |

What follows for the code:

- The package (`PKGBUILD`) carries no third-party graphics binary. The only
  download is OptiScaler's release, from GitHub, after the user's Apply.
- The AMD neural backend is `managed: false` in the model
  (`graphics/backend.rs`): no transaction, no manifest entry, no Restore.
  The page shows what is missing, what was found, and the official page.
- Nothing fetches or suggests a source for NVIDIA's neural-rendering model.
  The requirement is stated ("your own copy … beside the game") and left to
  the user.
- The reference tools this design learned from (DLSS5oneclick, MIT;
  DLSS-5-MANAGER, no open-source license) were read for their architecture
  only; no code was taken from either, and none of their leaked-DLL,
  NGX-gate or GPU-spoofing paths exists here.
