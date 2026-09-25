# Final validation — hybrid Intel + NVIDIA laptop

The audit's completion checklist, item by item, with the evidence behind
each answer. **Yes** means checked on the machine; **Partly** and **No** say
what is missing. Branch `test/hybrid-intel-nvidia`, commits from `b85c324`
to the one that adds this page.

## Checklist

| Item | Status | Evidence |
|---|---|---|
| Compiles without errors | Yes | `cargo build --workspace`; `makepkg` from the branch (release build, translation check) |
| Tests pass | Yes | 558 unit and integration tests; `tests/daemon-authorization.sh` PASSED (every privileged method refused with Polkit unreachable; path traversal refused, nothing written) |
| Clippy clean | Yes | `cargo clippy --workspace --all-targets` (pedantic): 0 warnings; `cargo fmt --check` |
| UI works | Yes | dev build run on the laptop; Details and Home checked with the game running (screenshots) |
| Daemon works | Yes | Turbo off/on through the real D-Bus → helper → Polkit path, four times in the Turbo session |
| D-Bus works | Yes | same; session environment set over the user manager's D-Bus and read back |
| Polkit works | Yes | `allow_active=yes` actions ran without a prompt; `auth_admin_keep` actions (profiles, scx_loader) raised a prompt, as designed |
| falcond works | Yes | profile activated for the game and deactivated after it, in the journal and in `/tmp/falcond_status`; once (right after falcond started) it kept its general `Proton` profile instead of `SOTTR.exe` |
| Turbo really switches | Yes | off: falcond stopped and disabled; on: running with 10 profiles; the report reads the system |
| State restored correctly | Yes | power profile, governor and falcond profile before / in game / after: balanced–schedutil / performance–performance / balanced–schedutil; Booster journal cleared on Turbo off; OptiScaler and lsfg-vk entries restored; Steam launch options and game registry put back |
| Installed games detected | Yes | library scan (only installed games since `7103a02`) |
| Tomb Raider detected | Yes | `SOTTR.exe`, AppID 750920, Proton Experimental, VKD3D-Proton |
| Correct game process identified | Yes | Steam runtime helpers and processes without a command line are no longer taken for the game (unit tests from real trees) |
| GPU the game really uses | Yes | NVML graphics-context list; SotTR's pid is in it, `vkcube` on Intel is not; Details: "Renders Shadow of the Tomb Raider" on the GTX |
| Intel detected correctly | Yes | card1 / renderD128, i915, drives eDP, actual clock read |
| NVIDIA detected correctly | Yes | card0 / renderD129, GTX 1050 Ti Mobile, no DLSS |
| PRIME validated | Yes | `glxinfo` through a BiGame-mode launch plan: NVIDIA's OpenGL instead of Intel; `DRI_PRIME=1` shown to give zink |
| NVIDIA telemetry | Yes | NVML matches `nvidia-smi`; clock, load, temperature, VRAM, P-state and power-limit reason (board power not reported by this GPU) |
| sched_ext tested | **No** | detection, scheduler list and scx_loader availability fixed and checked; switching a scheduler needs administrator authentication that could not be given unattended, so no scheduler was measured |
| power-profiles-daemon tested | Yes | ownership of the governor, the BigLinux companion, game cycle |
| Gamescope tested | Partly | options read from the installed 3.16.28; the game ran inside Gamescope (720p → 1080p FSR) rendering on the GTX; `--prefer-vk-device` found to break nested Gamescope on this laptop and removed; no valid performance run |
| MangoHud tested | Yes | frame logs for every run; used as the presented-frames counter |
| DXVK / VKD3D identified | Yes | VKD3D-Proton for DX12, DXVK for DX11, from the game's mappings; versions read from Proton |
| lsfg-vk tested | Yes | 1.0.0 from community-extra, installed for the user for the test; configuration loaded, layer mapped in the game |
| Local Lossless.dll validated | Yes | 3.2.2.0, used in place, never copied |
| Frame generation proven | Yes | lsfg-vk x2: presented 41.8 → 57.5 fps, repeatable; OptiScaler's: 60.7 presented, with Xid errors |
| Base and generated FPS told apart | Yes | the game's own count (rendered) and MangoHud (presented) kept in separate columns everywhere |
| OptiScaler tested where compatible | Yes | FSR 3.1 from XeSS installed, measured (+12 %), repaired and restored through the app's transaction |
| Applied settings verified | Yes | Booster knobs read back; session environment read back; lsfg-vk entry read back and confirmed by lsfg-vk's own log; launch options read back |
| Useless or wrong settings fixed | Yes | [UX_AUDIT.md](UX_AUDIT.md) |
| Regressions checked | Yes | full suite and package build after the last change |
| Benchmarks have real data | Yes | `bigame-engine/benchmarks/2026-09-25-sottr-gtx1050ti-*` |
| Final documentation | Yes | this page and the ones it links |

## Measured results

| Change | Result |
|---|---|
| OptiScaler FSR 3.1 in place of the game's XeSS | **+12 %** (40.6 vs 36.2 fps) |
| lsfg-vk x2 | **+37 % presented, −30 % rendered**, worse pacing |
| OptiScaler frame generation | 60.7 fps presented; **unstable** (Xid 69, Xid 31) |
| Turbo (performance profile and governor) | **no measurable difference** (38.0 vs 38.0 rendered) |
| DX11 instead of DX12 | **slower** (33.9 fps, 1 % low 7.7) |
| Async compute off | **slower** (33 vs 40) |
| XeSS off at 60 % resolution vs XeSS Performance | faster (40 vs 36) |
| BiGame-mode's own cost, Details open in the background | 1.63 → **0.46 %** of one core |

## Pending — what could not be validated here

- **Battery**: this laptop has none.
- **sched-ext schedulers**: need administrator authentication per switch.
- **Gamescope performance**: functional only; the benchmark navigation does
  not handle the scaled menu.
- **RTX paths** (DLSS, DLSS frame generation), **X11**, Mesa discrete GPUs
  (`DRI_PRIME`): not present on this machine.
- **Latency**: no instrument.
- **falcond keeping its `Proton` profile** once, right after it started: an
  upstream behaviour, reported here, not changed.
- **Health messages** are English only.
- The package must be installed (`sudo pacman -U`) for the fixes to reach the
  installed application; everything above ran from the branch's builds.

## Architecture recommendations

- **Measure from the UI for Steam games.** Steam games have no launch command
  BiGame-mode can wrap; a Proton game's built-in benchmark driven by the
  application (as done here with a virtual keyboard) or MangoHud through
  Steam's launch options would let the Benchmark page answer "did it help?"
  for the games people actually play.
- **Keep rendered and presented frames apart in the data model**, not only
  in documents: an outcome should carry which counter it came from, and a
  frame-generation arm should never be compared with a plain one on
  presented frames alone.
- **Re-plan on plug/unplug** (UPower), with the Booster's journal as the
  record of what to undo.
- **Treat per-game GPU choice as a launch-options feature** for Steam games,
  written with Steam closed, backed up and read back — the same pattern the
  Diagnostics fix for broken launch options already uses.
- **Stop storing BiGame-mode's own per-game settings inside falcond's
  profile files**; `game_settings` already exists for that, and it would
  make the "one setting, one owner" rule hold for files too.
