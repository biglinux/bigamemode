# AAA benchmarks

What the four installed commercial titles measured on 2026-09-23, and how they
were made measurable at all. Same machine as [12-FINDINGS.md](12-FINDINGS.md):
Ryzen 7 5700G, Radeon RX 9060 XT (RDNA 4, `amdgpu`), kernel
7.2.6-x64v3-xanmod1, Mesa 26.2.2, Proton Experimental, KDE Plasma on Wayland.

---

## Objective

The previous pass closed with one real workload, SuperTuxKart, and the note
that the AAA benchmarks "cannot be started unattended". A finding that rests
on one kart racer is a finding about one kart racer. The objective here was to
get a real AAA title through the same method — alternating arms, warm-up
discarded, significance tested — and see whether the result survives.

## How a menu-only benchmark became measurable

Two things were found that the earlier notes had not.

**The games already record every frame.** Crystal Dynamics' engine writes a
per-frame table (`*_frametimes_*.txt`) beside a summary whose `Settings:`
block lists every graphics option of the run; Cyberpunk 2077 writes
`frames.csv` and `summary.json`. That is better evidence than an overlay can
give: the engine times its own frames, over exactly the benchmark and nothing
else, with nothing injected. `bigame-core/src/benchmark/native.rs` reads both
formats. Its averages match the games' own printed figures exactly (34.6,
133.7, 131.5, 222.8 fps), with one deliberate exception below.

**Shadow of the Tomb Raider reruns its benchmark on a key.** Its results screen
offers `[R]`. One launch therefore serves a whole session, with the game loaded
and warm for every run after the first. `scripts/bench-game.sh` sets the arm
over the daemon's D-Bus interface, presses the key — held for 0.25 s, because a
tap from XTEST falls between two of the game's input samples, and only while
the game holds keyboard focus — waits for `[Benchmark] Benchmark stopped` in
the game's log, and collects the files. `bench_native_report` judges the result.

### Scene transitions

The benchmark loads between its sections, and the first pass of a launch
delivers that load as one frame: **7.8 s** in a real run, which is why the game
prints `Min FPS: 0.0`. Left in, that single frame *is* the 0.1 % low, and its
length depends on the disk cache rather than on anything being compared.
Frames of 1 s or longer are therefore counted as transitions and set aside,
and the count is reported. This is the only place the computed average differs
from the game's (81.8 against 77.9 in that run — exactly the 7.8 s).

### Two bugs found on the way

- **Two prefixes.** `compatdata/750920` exists in both Steam libraries. The one
  in the home library is stale; the game, and its results, are on the games
  disk. The first harness run found the wrong one. Both the harness and the
  provider now take the prefix from the library holding the game's manifest.
- **Summary files end in a NUL byte**, which makes `grep` treat them as binary
  and print nothing. Rust's reader is unaffected; the harness uses `grep -a`.

---

## Shadow of the Tomb Raider — GPU-bound

3440×1440, High preset, TAA, VSync off; Windows build on Proton Experimental,
DX12 through VKD3D-Proton. The game's own breakdown puts it at 99 % GPU limit
(CPU game 117 fps, CPU render 217 fps, GPU 84 fps).

Arms, all through the product's daemon:

| arm | power profile | governor / EPP | GPU DPM |
|---|---|---|---|
| `rest` | performance | performance / performance | auto |
| `gpu_dpm_level` | performance | performance / performance | **high** |
| `baseline` | balanced | powersave / balance_performance | auto |

`rest` is this machine's own resting state, and the reference for verdicts.

### Clean session — `benchmarks/2026-09-23-sottr-power-clean/`

Warm-up plus three rounds, arms rotated, nothing else running.

| metric | rest | gpu_dpm_level | change | verdict |
|---|---:|---:|---:|---|
| average fps | 89.4 | 82.2 | **−8.0 %** | slower; spread 0.1 %, Welch t = 199.8 |
| 1 % low | 65.0 | 60.1 | **−7.6 %** | slower; spread 2.1 %, Welch t = 5.3 |
| 0.1 % low | 50.2 | 43.4 | −13.5 % | not judged: ~14 frames per run, 6–11 % run-to-run |

| arm | core clock | board power | temperature |
|---|---:|---:|---:|
| rest | 3246 MHz | 163 W | 64.6 °C |
| gpu_dpm_level | 2640 MHz | 103 W | 58.7 °C |

The mechanism is the one [12-FINDINGS.md](12-FINDINGS.md) found in
SuperTuxKart, now in an AAA title under Proton: `high` pins the highest fixed
DPM state and takes the firmware's boost out of the loop, so the card runs
600 MHz slower and leaves 60 W of its budget unused while equally busy.

### First session — `benchmarks/2026-09-23-sottr-power/`

Four rounds of all three arms. Average frame rate:

| arm | runs | mean | verdict against rest |
|---|---|---:|---|
| rest | 88.4 88.2 89.3 89.1 | 88.8 | — |
| gpu_dpm_level | 81.3 80.2 82.3 81.9 | 81.4 | **−8.3 %**, Welch t = 13.9 |
| baseline | 88.3 88.2 89.2 89.2 | 88.7 | −0.1 %, no difference |

**The distribution default is as fast as the performance profile here.** In a
GPU-bound game, the power profile, the CPU governor and EPP have nothing to
give, and the numbers say so to a tenth of a percent.

**Its lows are contaminated, and it says so.** The lab VM turned out to be a
libvirt guest *on this machine*, and package installs and a Phoronix build in
it overlapped three runs. Those runs, and only those, contain freezes the
others do not — 3.6 s and 2.0 s in `baseline` run-02, 1.1 s in `baseline`
run-04, 333 ms in `gpu_dpm_level` run-02 — at wall-clock times matching the VM
work. The average is robust to that; the lows are not. That is why the clean
session exists, rather than the three runs being quietly dropped.

Every run, in every arm, has a hitch cluster at 40–43 s into the pass. It is in
the benchmark's content, not the machine.

### What changed as a result

[`plan.rs`](../bigame-engine/bigame-core/src/booster/plan.rs) no longer
proposes `gpu_dpm_level=high` by default. It had been measured slower in two
unrelated workloads and never faster, and a setting whose name promises speed
and whose every measurement says the opposite is not a default. A calibration
that finds it faster on some machine turns it back on there. On this machine
the Booster now reports:

```
MeasuredHarmful { knob: "GPU power level (card1)",
  detail: "measured on this machine against 2026-09-23-sottr-power-clean:
           8.0% slower, above the 0.1% run-to-run spread and significant at
           95% (Welch's t = 199.75 against a 2.78 threshold)" }
```

### A deployment gap this exposed

The Booster **installed on this machine** applied `high` at 18:52 anyway. The
installed package was built at 10:58; the planner learnt to respect the
calibration at 17:07. So every AAA run taken by hand today — Rise at 18:57,
Shadow at 19:18, Cyberpunk at 19:55, Superposition after — ran with the card in
the state measured to be slower. Shadow's 19:18 run (81.8 fps) sits exactly on
this session's `gpu_dpm_level` arm. The fix is to rebuild and reinstall the
package.

---

## Shadow of the Tomb Raider — CPU-bound

`benchmarks/2026-09-23-sottr-cpu-bound/`. Same launch, with the render scale
("Modificador de resolução") at its minimum: output stays 3440×1440, the GPU
drops to ~84 % busy, and the frame rate rises to ~116 fps, so the CPU side now
limits it. This is the case the CPU knobs exist for. GPU DPM `auto` in every
arm.

| arm | power profile | governor / EPP | runs (avg fps) |
|---|---|---|---|
| `baseline` | balanced | powersave / balance_performance | 116.6 117.2 *109.1* |
| `cpu_governor` | balanced | **performance / performance** | 116.0 115.1 *107.8* |
| `rest` | **performance** | performance / performance | 117.7 116.4 116.0 |

**Result: no difference above noise, even CPU-bound.** Rounds 1–2, which are
clean, put all three arms within 1.5 % of each other in no consistent order.
On this `amd-pstate-epp` CPU, raising the governor, EPP or power profile buys
nothing measurable in this title.

**Round 3 is interference, and the data shows why.** A continuous slowdown
from about 22:14:30 to 22:17:30 covers the *end* of `baseline` run-03 and the
*start* of `cpu_governor` run-03 — two arms, split by a configuration change,
with CPU clocks unchanged throughout. An arm cannot cause a slowdown that
starts before it is applied. Its source was not found: not the VM (idle), not
this session's tools (idle), not the BigLinux governor service (below). The
two runs are kept and marked, not deleted.

### A second governor authority, inert here

`power-profiles-daemon-biglinux-cpufreq.service` is triggered by a path unit
on every write to power-profiles-daemon's `state.ini` — so by every profile
change, including the Booster's. It maps profiles to governors and calls
`cpupower frequency-set`, but only when `schedutil` is offered. On
`amd-pstate-epp`, which offers only `performance` and `powersave`, it does
nothing to the governor. On an `acpi-cpufreq` machine it would be a second
writer of the same knob the Booster sets, racing it after every profile change.
That is worth knowing before anyone reports "the governor did not stick".

---

## The product's own overhead

§55 asks for the application's cost to be measured rather than assumed. It was,
and it was not small.

### What the running UI was doing

With falcond's generic `Proton` profile active, the dashboard's 1-second status
poll ran `pgrep -f Proton` three times, `pgrep -f gamescope` once, and
`timeout 0.2 grep` twice for each of the 7 matching processes: **about 32
process creations a second**, out of 55/s on the whole machine, for as long as
the game ran. For scale: the telemetry sampler's 1400 forks per run once raised
SuperTuxKart's run-to-run spread from 1.4 % to 6 %. This was about 6000 per
Shadow pass.

It also produced two wrong answers:

- `is_gamescope_running()` used `.status()`, so `pgrep`'s output went to the
  journal: **7 653 lines today**, each the same PID.
- That PID was a **zombie**. The dashboard's Gamescope launcher dropped the
  `Child` handle without waiting, so the Gamescope it had launched stayed
  `<defunct>` for 6 h 43 min — and `pgrep` counts zombies, so the dashboard
  reported Gamescope as running throughout.

### Measured

`benchmarks/2026-09-23-sottr-ui-overhead/`, CPU-bound as above, `rest` state.
The UI was frozen with `SIGSTOP` rather than closed, so nothing it owns was
restored or torn down, and resumed with `SIGCONT`.

| arm | avg fps | 1 % low |
|---|---|---|
| UI polling | 114.8 115.8 114.4 | 68.6 67.8 67.6 |
| UI frozen | 116.4 116.6 115.7 | 68.8 69.1 68.7 |

**+1.1 % with the UI frozen, every frozen run above every polling run — and not
significant** at three runs a side (Welch's t = 2.38 against 2.78). It is
reported as a suggestion, not a gain. The fix did not need it: the false
"Gamescope active", the zombie and the journal flood are defects on their own.

### Fixed

`bigame-core::processes::find_by_cmdline` and `maps_contain` read `/proc`
directly, skip zombies and never match themselves; two full scans take 15 ms.
The dashboard finds the game's processes once per tick instead of three times,
the Logs view uses the same helpers, and every child the UI launches is reaped
off the UI thread. Verified live: `pgrep -f gamescope` still reports the old
zombie, and `find_by_cmdline("gamescope")` does not.

The installed UI keeps doing all of this until the package is rebuilt.

---

## A mistake of mine during the session

Restoring the render scale after the CPU-bound sessions, my key presses landed
on the wrong row — the rerun key had reset the menu's selection to the top —
and switched **DirectX 12 off**. The game asks for a restart before that takes
effect, so it had not been applied; it was switched back before anything was
confirmed, the render scale was then restored, and a further pass was run to
check. Its `Settings:` block is **identical** to the clean session's. The rule
it teaches is already in the harness for the rerun key and should be for
everything else: screenshot, check the highlighted row, then press.

---

## The other titles

| title | status | detail |
|---|---|---|
| Cyberpunk 2077 | **Read, not driven** | Its result format is parsed and cross-checked (34.6 fps at 3440×1440 RT Ultra + FSR 2, matching the game's own figure). The benchmark has no rerun key, so a session needs a person per run. Its `summary.json` states frame generation explicitly, and the reader records it; a run with generation on is refused by the report. |
| Rise of the Tomb Raider | **Not yet measured** | Now the Windows build under Proton (the native Feral build was removed at 19:22). The Feral build's output was parsed (222.8 / 131.5 / 133.7 fps across its three scenes); where the Windows build writes has not been seen, so the provider does not guess. |
| Tomb Raider (2013) | **NOT TESTED** | As before. |
| Unigine Superposition | **One manual run** | 1080p Medium, OpenGL: score 19951, 149.2 fps average, with the GPU at DPM `high`. Its launcher reports the card as "ASRock Microsoft Basic Display Adapter 512 MB" — its 2019 hardware table, not a detection problem here. |

## The user's own launch options

Shadow's Steam launch options are
`WINE_FULLSCREEN_FSR=1 ENABLE_VKBASALT=1 RADV_PERFTEST=afmf %command%`.
Checked against the running process rather than assumed:

- `ENABLE_VKBASALT=1` — **active**: `vkbasalt.so` is mapped into the game and
  its config runs CAS sharpening. A real, if small, GPU cost.
- `RADV_PERFTEST=afmf` — **inert**: the string does not occur in Mesa 26.2's
  RADV. AFMF is a feature of AMD's Windows driver.
- `WINE_FULLSCREEN_FSR=1` — **inert here**: a Proton-GE patch; this title runs
  on Proton Experimental.

They were left exactly as the user set them, identical in every arm, so they do
not bias any comparison above. Measuring vkBasalt's cost is possible in-session
with its toggle key, but a toggle whose state cannot be read back could produce
a false "no difference", so it was not done.

## Background load during the sessions

- `big-screen-monitor-display.service`, a root `python3` system monitor, used
  about 38 % of one core continuously. Present in every arm; reported, not
  touched.
- falcond's generic `Proton` profile matched the game at launch and stayed
  active throughout (`performance_mode`, `idle_inhibit`). Per-run `state.txt`
  confirms it did not override any arm's settings.
- The game ran at nice −4. The source was not identified; it was constant.

## Limitations

- **One card.** Both regressions are on one RX 9060 XT. The calibration is
  bound to the machine fingerprint so it is never carried elsewhere, and the
  planner change makes the knob opt-in by evidence rather than removing it.
- **One AAA title driven.** Cyberpunk and Rise need a person per run.
- **Latency not measured.** Nothing here measures input-to-photon time.
