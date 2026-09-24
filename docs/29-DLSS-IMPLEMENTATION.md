# 29 — AI Graphics implementation

What was built on `feature/ai-graphics-dlss`, where it plugs into the
existing code, and what was removed. Development documentation; the
application does not read it. Architecture: [28](28-DLSS-ARCHITECTURE.md).

## User flow

1. **Profiles → a game's card → ⋮ → AI Graphics…**, or the profile wizard's
   new step *"Enable AI Graphics for this game?"* (Recommended · Advanced… ·
   Not now), which opens the same page after the profile is saved.
2. The page analyses the game (tens of milliseconds) and shows, top to
   bottom: what is happening now; the plan's one-line summary and standing;
   its steps — what BiGame-mode would install, what to choose in the game's
   own menu, what it would turn off for this game; the files that would
   change. *Choose yourself* (upscaler, frame generation, experimental) and
   *What was found* (API with its confidence and evidence, graphics card and
   driver, the game's own upscalers with versions, executable and
   architecture, DLL slots and their owners, anti-cheat) are collapsed.
3. **Apply** downloads the pinned OptiScaler release if the cache does not
   have it, checks its SHA-256, builds the payload and applies it as a
   transaction. It refuses while the game runs.
4. The next launch loads it. The **Home** card shows the real status while
   the game runs; **Diagnostics** lists every game BiGame-mode changed.
5. **Repair** puts back files that went missing; **Restore Game Graphics**
   removes what BiGame-mode placed and puts every original back; the header
   button saves a support report to Downloads.

Nothing is applied from the wizard, from a recommendation, or at launch: only
the Apply button changes a game's files.

## Integration points

| Where | What changed |
|---|---|
| `launcher.rs` | `apply_graphics_disables`: a game with an installed manifest launches without Wine FSR and without a Gamescope render size (a second upscaler each), for that launch only. The `OptiScaler` staging functions and the AFMF environment variable are gone. |
| `models/mod.rs` | `FrameGenSettings` is `enabled` + `backend` (`none` / `lsfg_vk`); old `optiscaler`/`afmf` values load as `none`. |
| `fg.rs` | `layer_installed()` (moved from the dashboard, used by Video too). |
| `hardware.rs` | `GpuVendor` serialises. |
| `game_settings.rs` | per-game user settings file with `[ai_graphics]`. |
| `bigame-ui/src/views/ai_graphics.rs` | the page. |
| `profile_wizard.rs` | AI Graphics step; CPU governor step removed. |
| `profiles.rs`, `widgets/game_card.rs` | card entry carries the AI Graphics target; *AI Graphics…* in the card menu. The card menu's popover was unparented before its item's action ran, so **no** item of that menu worked (Edit, Delete, Measure); fixed. |
| `home.rs` | status line on the running game's card. |
| `diagnostics.rs` | AI Graphics section. |
| `video.rs` | frame-generation group reduced to what works: lsfg-vk on/off when installed. |
| `dashboard.rs` | decorative OptiScaler/AFMF status and the launch-time staging call removed. |
| `locale/extract-strings.py` | collects `N_()` too; reads Rust line continuations and `\u{…}`/`\xNN` like Rust. |

## Removed

| What | Why |
|---|---|
| `launcher::stage_optiscaler_dlls`, `maybe_stage_optiscaler`, `resolve_optiscaler_source` and the dashboard call | copied `dxgi.dll`, `nvngx.dll`, `_nvngx.dll`, `OptiScaler.ini` over whatever the game had, from any folder, without backup, record or removal |
| `FrameGenBackend::OptiScaler`, `::Afmf`; `optiscaler_enabled`, `optiscaler_source_dir`, `afmf_*`, `mode`, `osd_enabled` | the OptiScaler backend was the staging above; AFMF set `RADV_PERFTEST=afmf`, which RADV 26.2.2 does not have (no `afmf` string in `libvulkan_radeon.so`; AMD Fluid Motion Frames is a Windows driver feature); `mode` and `osd_enabled` were saved and never read |
| Dashboard `is_optiscaler_active`, AFMF detection, the frame-generation "conflict" badge | "active" meant any mapped file named `nvngx.dll` (also NVIDIA's own DLSS) or an environment variable; the conflict compared two placebo settings |
| Wizard CPU governor step | falcond never reads a per-game governor |

## Defects found and fixed on the way

- **SotTR's XeSS change was not applied** by the test automation until the
  game's *[E] Apply changes* and its keep-changes dialog were answered — list
  options in that menu need Apply, sliders do not. Found by reading the
  result files' settings block (`XESS=0`), not by trusting the menu.
- **Stray key presses toggled FidelityFX CAS** while XeSS greyed rows out and
  moved the focus; caught on the next screenshot, reverted before Apply
  (nothing was applied). Every later press was one key per screenshot.
- **Version reading**: a stray `VS_FIXEDFILEINFO` signature in NVIDIA's DLSS
  runtime read as `46863.0.46863.4696`; the reader now follows the resource
  directory to `RT_VERSION`.
- **Scan cost**: reading the first 64 MB of executables for their import
  table (Cyberpunk: 800 ms) → positioned reads of the table only (1.5 ms).
- **Transaction**: a path failing its check after another file had already
  been backed up left an orphan backup; every target is now examined before
  any copy.
- **i18n**: every string written with a Rust line continuation had a msgid
  the program never asked for.
- **Card menu**: see above.
