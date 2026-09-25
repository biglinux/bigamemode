# 34 — AI Graphics: audit against the specification, and the second pass

Branch `feature/ai-graphics-audit`, from `main` at `ef1986a`, 2026-09-24.
Development documentation; the application does not read it.

The first pass ([25](25-DLSS-RESEARCH.md)–[32](32-DLSS-FINAL-REPORT.md))
was built and verified on one machine: Ryzen 7 5700G + Radeon RX 9060 XT
(RDNA 4), 3440×1440. This pass checked every item of the specification's
definition of done against the code, and ran the result on a second,
very different machine:

| | Test machine (first pass) | Lab laptop (this pass) |
|---|---|---|
| CPU | Ryzen 7 5700G | Core i7-7700HQ |
| GPU | Radeon RX 9060 XT (RDNA 4), Mesa | **Intel HD 630 + GeForce GTX 1050 Ti Mobile** (hybrid), NVIDIA 580.178.04 |
| Display | 3440×1440 | 1920×1080 laptop panel |
| Proton | Experimental | Experimental |

A hybrid laptop with a pre-RTX GeForce is exactly the case the first pass
could not test, and it found real defects.

Statuses: **VERIFIED** (observed on a real machine), **TESTED** (automated),
**IMPLEMENTED** (compiled, not observed end to end), **NOT TESTED**.

## 1. Defects found and fixed

| # | Defect | Where it showed | Fix |
|---|---|---|---|
| 1 | Games on a hybrid Intel + NVIDIA laptop were planned for the **iGPU**: a GPU counted as discrete only if the driver published `mem_info_vram_vendor`, which only `amdgpu` does. The unit test passed because it injected `discrete: true`. | `graphics_plan SOTTR.exe` on the lab laptop: *gpu: Kaby Lake-H GT2 [HD Graphics 630]* | `hardware::looks_discrete`: NVIDIA on PCI is always discrete; Intel is discrete off the root bus; AMD as before. Test with the lab laptop's real sysfs values. |
| 2 | The render GPU of a running game was the **first** `renderD*` it had open; a game on the NVIDIA proprietary driver renders through `/dev/nvidia0` and may hold the iGPU's node too. | code review, then VERIFIED live | `running::choose_render_gpu`: `/dev/nvidiaN` first (minor → PCI slot from `/proc/driver/nvidia/gpus`), then the non-boot card among several render nodes. Live: SotTR running → `render_card: card0` (the GTX). |
| 3 | **DLSS was recommended on any NVIDIA card**, including a GTX, which cannot run it ("the game's own DLSS … runs natively on this GPU"). | plan logic; the game itself greys *NVIDIA RTX DLSS* out on the GTX | `report::nvidia_dlss` from the PCI database name (RTX brand, chip codes; GTX 16 = TU116/117 excluded; FG from Ada/Blackwell); unknown models are unknown, never "yes". The plan uses DLSS only on a confirmed RTX, refuses DLSS/DLAA in Advanced with the reason. |
| 4 | `install` and `repair` always used the **recommended** OptiScaler release, ignoring a pinned version; after an update, Repair would have fetched different files than the manifest's hashes. | code review; VERIFIED fixed (repair of 0.9.3 put back 0.9.3's `dxgi.dll`) | `versions::resolve` / `versions::for_installed` (by archive SHA-256). |
| 5 | `Failed to load amdxcffx64.dll` was reported as **Failed**, though OptiScaler logs it as a warning and goes on with FSR 3.1 — the normal case under Proton without AMD's Windows driver DLL. | `FSR4Upgrade.cpp` (v0.9.4) | Not an error; recorded as `amdxcffx64 = Some(false)` → "FSR 3.1". |
| 6 | "OptiScaler takes over the game's XeSS — verified … **on this machine**" was shown on every machine. | lab laptop | "on BiGame-mode's test machine". |
| 7 | OptiScaler frame generation on + lsfg-vk on: the plan listed lsfg-vk to turn off, the **launch did not**. | code review | `launch_disables` reads the game's `OptiScaler.ini`; the launch sets `DISABLE_LSFG=1` (lsfg-vk v1.0.0 layer manifest). |
| 8 | With OptiScaler installed, the page said **"the game's own FSR"**: the scan counted the AMD DLLs BiGame-mode itself placed. | VERIFIED through the UI | `report::without_added`: files the manifest added are not the game's; files it replaced still are. |
| 9 | `Layer::OptiScaler` was written `opti_scaler`; a hand-written `optiscaler` made the whole settings file unreadable and the page silently fell back to defaults. | UI test | written `optiscaler`, `opti_scaler` still reads; an unreadable file is logged. |
| 10 | Conflict rows showed Rust enum names (`GamescopeUpscaling + OptiScalerUpscaler`), untranslated. | code review | `tech_name`, translated. |
| 11 | An emptied game folder was left in the state after Restore. | VERIFIED | removed at the end of the rollback, after staging and backups. |
| 12 | A profile export did not carry the AI Graphics choice. | code review | `[ai_graphics]` in the export (intent only), saved back on import. |
| 13 | **With OptiScaler installed, the game exited 4 s after start on the GTX.** OptiScaler turns its DLSS path on for any NVIDIA GPU when the game ships `nvngx_dlss.dll` (`dllmain.cpp`, v0.9.4: *Running on Nvidia → Enabling DLSS*); the driver's NGX refused (`NVSDK_NGX_D3D11_Init_Ext … BAD00001`) and the process died in the game's D3D11 launcher. | VERIFIED live on the lab laptop; OptiScaler.log | On an NVIDIA card that cannot run DLSS the ini gets `[DLSS] Enabled=false`, so OptiScaler takes its AMD/Intel path (*DLSS.Enabled: false … disabling DLSS*). Relaunched: launcher, game and benchmark ran; BiGame-mode's status *Active (fsr31), FSR 3.1* from the log. |
| 14 | A measured gain whose 1 % low could not be compared (runs varying above the 5 % ceiling) counted as "better": the plan would have **promoted OptiScaler and written "with the 1 % low no worse"** — which this machine's own session showed (+13.4 % average, floor inconclusive). "Runs each" was also the smaller arm's count. | the GTX benchmark ([31](31-DLSS-BENCHMARKS.md#lab-laptop-geforce-gtx-1050-ti-mobile)) | `Learned::better` needs the floor shown no worse; `faster_floor_unknown` reports the gain and leaves the choice to *Choose yourself*; both arms' run counts shown; the measurement replaces the general reason instead of contradicting it. |

## 2. Added

- **OptiScaler versions** (§21–22): Recommended (tested) / Latest stable /
  Pinned; the release list from GitHub under the download rules, cached,
  refreshed at most daily; a newer release offered — **Update · Skip · Keep
  this version** — never applied by itself or during a launch; **Go back**
  to the version before the last update. An update downloads and checks both
  releases before the game changes and puts the previous one back if the new
  one fails to apply.
- **Measured here** (§43): benchmark arms recorded locally
  (`bench_native_report --record-graphics`), refused if the session was
  measured on another machine; the plan promotes OptiScaler to Recommended
  only when the runs prove it faster with the 1 % low no worse, and says so
  when they prove it is not.
- **Game list** (§44): carried in the program plus the user's own; API
  default, prefer, block, tested version. One carried entry (SotTR), with
  evidence. Cannot unblock anti-cheat.
- **Diagnostics** (§29): the graphics card games run on, its driver, whether
  it runs DLSS, whether it is one of several; each game's OptiScaler version.
- **Two-GPU note** in every plan until the game runs and the render GPU is a
  fact.

## 3. Definition of done (§57), item by item

| Item | Status | Evidence |
|---|---|---|
| Compiles without errors | VERIFIED | `cargo build --workspace --all-targets` |
| `cargo test` passes | TESTED | 556 tests, 0 failures (§7) |
| `cargo clippy` without relevant regressions | VERIFIED | 0 warnings, workspace, all targets, pedantic (the two pre-existing `fg.rs` warnings fixed too) |
| Old profiles keep working | TESTED | serde defaults; tests for configs without `[ai_graphics]`, without `skipped_update`, with `opti_scaler` |
| New profile can enable AI Graphics optionally | VERIFIED (first pass) | wizard step *Recommended · Advanced… · Not now*; unchanged |
| Detects hardware | VERIFIED | both machines; hybrid laptop now correct (defects 1–3) |
| Detects the game | VERIFIED | SotTR from Steam's records, executable, 64-bit |
| Detects the API when possible | VERIFIED | SotTR: *Likely DX12* from files → *Detected* with the game list → *Fact* from the running game's VKD3D-Proton |
| Detects existing DLSS/FSR/XeSS | VERIFIED | SotTR: DLSS 2.3.2.0, XeSS 1.1.0.21; BiGame-mode's own files excluded (defect 8) |
| OptiScaler applied safely | VERIFIED | install 0.9.3 / update 0.9.4 / go back / repair ×2 / remove; UI and CLI |
| Backup works | TESTED + VERIFIED (first pass, replaced files) | SotTR ships no `dxgi.dll`, so no file was replaced this pass |
| Rollback works | TESTED + VERIFIED | failed-apply tests; *Go back* live |
| Remove works | VERIFIED | 189 files byte-identical after the CLI cycle and after the UI cycle |
| Manifest works | VERIFIED | hashes, previous version, generated log |
| Conflicts detected | TESTED | rules + plan tests; proxy-slot owner stop |
| lsfg-vk does not stack with another FG | TESTED | defect 7; lsfg-vk not installed here: live NOT TESTED |
| Gamescope does not double-upscale | TESTED | `launch_disables`, launcher tests |
| No automatic injection with anti-cheat | TESTED | fixtures; no anti-cheat game installed on either machine (Arc Raiders and Metal Slug Awakening folders here are empty remnants): live NOT TESTED |
| Dashboard shows what is really active | VERIFIED (first pass) | Home status from maps + log; unchanged path |
| Diagnostics explains failures | VERIFIED | GPU row live; status texts (defect 5) |
| Texts ready for gettext | VERIFIED | `extract-strings.py --check` clean; pt_BR complete (861) |
| Documentation updated | done | this file; 27, 28, 30, 31, 32 |
| Comparative benchmark when possible | VERIFIED | §6: A-B-A on the GTX, +9–16 % average for OptiScaler FSR 3.1 over the game's XeSS, 1 % low inconclusive |

## 4. Specification items deliberately not done

- **Other proxy slots** (`winmm.dll`, `version.dll` …): they need a
  `WINEDLLOVERRIDES` entry, which for Steam games means Steam's launch
  options — changed only with Steam closed. `dxgi.dll` works with none; when
  it is taken, the plan names the owner and stops.
- **ReShade/RenoDX installation**: detected and planned around, not
  installed (ReShade asks that users be sent to its site; RenoDX does not
  support Linux officially) — unchanged from [26](26-DLSS-LICENSE-AUDIT.md).
- **Telemetry**: none. The measurement record is local.

## 5. Tests

`cargo test --workspace`: see §7 for the final count. New tests this pass
cover: discrete detection from each vendor's evidence; render-GPU choice on a
hybrid laptop and under `DRI_PRIME`; DLSS/DLSS-FG capability for 19 real PCI
database names; GTX never told DLSS; the two-GPU note; FSR 3.1 proven and FSR 4
never claimed from OptiScaler's own messages; version ordering, release-list
parsing with real digests, policy resolution, the update offer, finding the
installed release by hash; outcomes parsing, verdicts (gain, noise, worse
floor, one run), the record; plan promotion and refusal from measurements, and
native DLSS on RTX not overruled; the game list (carried, user override,
broken user list, block/prefer, anti-cheat still wins); listed API filling in
only uncertainty; added files not counted as the game's; lsfg-vk off at launch
from the game's ini; profile export carrying the choice without paths; the
state folder removed after Restore.

## 6. Benchmark on the lab laptop

Full method and numbers in [31](31-DLSS-BENCHMARKS.md#lab-laptop-geforce-gtx-1050-ti-mobile).
Shadow of the Tomb Raider, DX12, 1080p High, XeSS Quality in the game's
menu, order A-B-A with a warm-up per launch:

| | avg fps |
|---|---|
| the game's XeSS, first launch | 15.9 · 15.7 · 15.5 |
| OptiScaler 0.9.4, FSR 3.1 from the game's XeSS (1280×720 → 1080p) | **18.2 · 18.3 · 18.3** |
| the game's XeSS, third launch | 16.6 · 16.8 |

+13.4 % against both XeSS launches pooled (Welch significant), between +9 %
and +16 % against either; the GPU clock was *lower* in the OptiScaler arm.
The 1 % and 0.1 % lows vary too much at ~16 fps to compare. The session is in
this machine's local measurements, and the planner reports the gain without
promoting it (defect 14).

## 7. Final state

- `cargo build --workspace --all-targets`: clean.
- `cargo test --workspace`: **556 passed, 0 failed** (524 core, 16 + 16 UI,
  0 doc). AI Graphics: 107 tests (97 at the start of the pass).
- `cargo clippy --workspace --all-targets` (pedantic, workspace lints):
  **0 warnings**.
- `cargo fmt --all -- --check`: clean.
- `locale/extract-strings.py --check`: up to date; pt_BR **863/863**.
- Shadow of the Tomb Raider on the lab laptop, after 3 installs, 1 update,
  1 go-back, 2 repairs, 3 removals (CLI and UI) and 11 benchmark passes:
  the **189 original files byte-identical** to the snapshot taken before the
  first install; the one new file is `vkd3d-proton.cache`, written by
  VKD3D-Proton when the game ran, not by BiGame-mode — and, not being in any
  manifest, left alone.
- Left on the machine on purpose: the benchmark session in this machine's
  local measurements (`~/.local/state/bigame-mode/graphics-outcomes.json`,
  real data the planner uses) and the two verified releases in the download
  cache (0.9.3, 0.9.4). Removed after the tests: the game's AI Graphics
  choice they had saved and the last run's kept `OptiScaler.log` — neither
  existed before.
