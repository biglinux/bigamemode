# 30 — AI Graphics tests

What was tested, how, and what was not. **TESTED** means automated (unit or
fixture), **VERIFIED** means observed on the reference machine (Ryzen 7
5700G, RX 9060 XT — RDNA 4, Mesa 26.2.2, Proton Experimental, KDE Wayland),
**NOT TESTED** means exactly that. Development documentation.

## Automated

`cargo test --workspace`: **511 tests, 0 failures**; `cargo clippy
--workspace --all-targets` (pedantic): 0 warnings; `cargo fmt --check` clean;
`locale/extract-strings.py --check` up to date. 97 of the tests are AI
Graphics'. Every file-handling test runs against real files in a temporary
folder (`tempfile`); no test touches a real game.

| Area | What the tests establish |
|---|---|
| PE reader | imports and delay-imports from a synthetic PE32+ and PE32 image; 32-bit told apart; garbage and every truncation prefix fail cleanly; a name pointing outside the file is skipped; version from `VS_VERSIONINFO` with a stray signature before it ignored; version read through the resource directory of a file |
| Scanner | a SotTR-like folder (DLSS + XeSS, EOS SDK not flagged); bare `nvngx.dll` is a shim, not DLSS; proxy owners read from contents and only beside the executable (ReShade, OptiScaler, unknown, Microsoft outside the exe folder ignored); the running process name settles the executable; anti-cheat by folder, file and executable name (EAC, BattlEye, XIGNCODE3, Ricochet, EA Javelin, Tencent ACE, PunkBuster, VAC by exe); symlinks not followed, depth bounded |
| Manifest | only plain relative paths; symlinks inside the game folder refused; round trip; a newer format refused; a manifest naming `../` rejected on load; stable, safe game keys; SHA-256 |
| Transaction | apply backs up, places, verifies; remove restores byte-for-byte and removes created folders; a binary changed by someone else is left; an edited config is kept as a copy; a file deleted by a game update gets its original back; a failure after the first file was placed rolls back; a target that cannot exist fails before any backup is written; an interrupted apply is rolled back by `recover`; nothing escapes the game folder (`../`, absolute, through a symlink); a second apply and case-insensitive duplicate slots refused; repair puts back only missing files; a damaged backup is never restored; the component's run-time log is removed with it but a pre-existing one is not |
| OptiScaler | section-scoped ini edits (replace in place, add to section, add section, other sections untouched); the SotTR/RDNA 4 configuration (`fsr31`, spoofing off, XeSS not replaced); DLSS input on AMD brings spoofing and fakenvapi, never the Agility SDK; DX11/Vulkan through interop; frame-generation keys and files; a `dxgi.dll` owned by another tool reported; archive listings with `..`, absolute paths, links or devices refused; a real zip unpacked only when clean; log parsing (version, slot, Wine, upscaler created, FSR 4 line, errors); GitHub API parsing (stable only, one `.7z`, sha256 digest, safe asset name) |
| Rules | two upscalers or two frame generators never pass; injection into anti-cheat games blocked; unestablished pairs are Unknown; every rule has a reason; no pair listed twice; problems sorted worst first |
| Planner | SotTR on RDNA 4 → FSR 4 from XeSS with the four files; older AMD → the game's own, no files; NVIDIA with native DLSS → nothing installed; anti-cheat blocks injection but still points at the game's own upscaler; 32-bit → native; DLSS-only → spoofing, Experimental; ReShade's `dxgi.dll` stops the plan; Gamescope/Wine FSR disabled for the launch; frame generation never automatic, Experimental when chosen, lsfg-vk disabled with it; native Linux game → explanation only; Off does nothing |
| Runtime status | `/proc` maps paths with spaces read whole; not running → Configured / FilesChanged; Active only when the log says an upscaler was created; a failure line wins; Starting within the grace period, NotDetected after; a log older than the process is not read as this run |
| Report | running game makes the API a fact; imports are Detected; a run-time-loaded renderer is Likely; no evidence is Assumed and says so; GPU names from the PCI database, RDNA generation |
| Config / settings | an old profile loads as Off; unknown fields ignored; portable (no paths); per-game settings file round trip, broken file reported not replaced, names that could escape the folder refused |
| Launcher | old `afmf`/`optiscaler` settings set nothing; graphics disables drop Wine FSR and the Gamescope render size for the launch only, the caller's settings untouched |
| Models | a `video.toml` from before AI Graphics loads (old backends → `none`) |
| Text | `%s` placeholders filled in order |
| Support | identifiers masked, log tail, safe file names |

## Integration, on the reference machine

| Case | Result |
|---|---|
| Proton DX12 game with native DLSS and XeSS (Shadow of the Tomb Raider) | VERIFIED end to end: scan (0.6 ms), plan, Apply from the UI, OptiScaler loaded as `dxgi.dll` with no `WINEDLLOVERRIDES`, `init successful for fsr31` at the game's XeSS render resolution, Home *Ativo (FSR)*, Diagnostics lists it, Restore refused while running, folder byte-identical to the pre-install snapshot after Restore (170 files hashed) |
| Proton DX12 game with every upscaler (Cyberpunk 2077) | scan 1.5 ms warm; DLSS 310.1, DLSS-G, DLSS-RR, Streamline 2.7.1, XeSS 2.0.1, XeSS-FG, FSR 3 all detected with versions; plan FSR 4 from XeSS (`bin/x64/`); `dbghelp.dll` beside the exe recognised as Microsoft's. **Not installed or launched** |
| Proton game with only DLSS (Rise of the Tomb Raider) | plan Experimental (spoofing + fakenvapi), as designed. Not installed |
| 32-bit Proton game without upscalers (Tomb Raider 2013) | plan Not recommended: no upscaler, 32-bit. Not installed |
| Native Linux Vulkan game | planner path TESTED; live: SuperTuxKart's Lutris record has no install folder, so no target — NOT TESTED live |
| Game with native DLSS on NVIDIA | NOT TESTED — no NVIDIA GPU |
| Game with an existing `dxgi.dll` (ReShade / DXVK / Special K) | TESTED with fixtures; NOT TESTED live — none installed here |
| Game with ReShade or OptiScaler already installed by hand | TESTED with fixtures (owner detection, slot refusal, `OptiScaler` recognised as its own) |
| Anti-cheat title | TESTED with fixtures; NOT TESTED live — deliberately: no protected game is a test bench |
| Interrupted apply | TESTED (`recover`); NOT TESTED by killing the process mid-apply on a real game |
| Profile wizard AI Graphics step | VERIFIED on screen (pt_BR) |
| Card menu → AI Graphics… | VERIFIED — and found that **no** item of that menu worked before (popover unparented before the action ran); fixed |
| Translations | VERIFIED: the page, wizard step, Home line and Diagnostics section in pt_BR; 502 of 810 strings translated (every string this work added; the 308 left predate it) |

## Not done

- Visual quality: screenshots at fixed wall-clock offsets land on slightly
  different camera instants, so no pixel comparison was possible; no
  artefact was visible at 1/3 scale in any of the three configurations.
  Inconclusive ([31](31-DLSS-BENCHMARKS.md)).
- Frame generation (OptiScaler's), RenoDX, ReShade, HDR: not installed by
  this pass; rules from upstream only.
- Lutris and Heroic games live; Gamescope nested with OptiScaler; X11.
