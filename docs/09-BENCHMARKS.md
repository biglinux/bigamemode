# 09 — Benchmarks

## 1. Status

**The benchmark engine is built and validated against real captures on the
reference machine. No game was benchmarked.**

Those are two different statements and the distinction matters. `benchmark.rs`
captures frametimes, computes the statistics, measures its own noise floor and
makes a defensible call — verified end to end with real MangoHud data and a real
A/B run. What has not happened is running it against an actual game, so the
application still reports:

```
Performance impact not measured
```

for every Booster activation. That remains correct until a per-game capture
exists.

## 2. Design

Three decisions shape the module.

**Frametime is the measurement; FPS is derived.** An averaged FPS counter
destroys exactly the information that matters. The validation below shows why:
one configuration averaged 7196 FPS and the other 6097, a 15% gap — but the 1%
low gap was 7.5%, and a run with a hitch every second reports the same average
as a perfectly smooth one.

**1% low is reported first.** It is what players perceive as smoothness, and it
is the metric a scheduler or power change is most likely to move. Average FPS is
reported last, because it is the easiest number to move without improving
anything.

**A difference smaller than the measured noise is not a difference.** The noise
floor is measured by comparing two baseline runs of the *same* configuration
against each other, never assumed. An engine that skips this step reports an
improvement every single time.

```rust
pub fn noise_floor(values: &[f64]) -> Option<f64>   // None with fewer than 2 runs
pub fn compare(metric, unit, before, after, direction, noise_floor) -> Outcome
```

`Direction` exists so that frametime going down and FPS going up are both
improvements, and `Regressed` is a first-class result — an engine that can only
report wins will eventually report one that did not happen.

## 3. Capture

Delegated to MangoHud, which is already a dependency and already writes
per-frame CSV. Nothing modifies the game or injects anything.

Two things about MangoHud 0.8.4 were established by trying them, and both
silently produce **no log at all** when wrong:

* The settings must reach MangoHud through `MANGOHUD_CONFIGFILE`.
  `MANGOHUD_CONFIG` with the same keys produced nothing.
* **`no_display=1` must not be set.** It suppresses the overlay and the CSV
  together — which is exactly the setting a benchmark would reach for first.

Both are recorded in `mangohud_config()` and the second is asserted by a test,
because the failure mode is an empty directory rather than an error.

The parser locates columns by name, not index, because MangoHud's column set
varies with what the machine exposes. `frametime` is milliseconds; `elapsed` is
nanoseconds. Rows that do not parse are skipped rather than fatal, and
frametimes outside a plausible range are discarded as logging artefacts.

## 4. Validation against real data

### 4.1 Parsing

A real capture — `mangohud vkcube` on the RX 9060 XT, 6 seconds, 959 frames — is
embedded in the tests. It asserts frametimes, GPU temperature, GPU power, and
that the nanosecond `elapsed` column yields 5.99 s.

### 4.2 A first A/B run that was inconclusive, and said so

The first attempt compared `performance` against `power-saver` with vkcube
vsync-locked to the 49.95 Hz display:

```
A_run1  599 frames  avg 50.0 fps  1% low 47.3 fps  median 19.96 ms
B_run1  599 frames  avg 50.0 fps  1% low 47.3 fps  median 19.98 ms

noise floor: 0.6%
  1% low: no measurable change
  P99 frametime: no measurable change
  Average FPS: no measurable change
```

Both configurations hit the refresh ceiling, so the workload could not reveal a
difference **even if one existed**. The engine reported no change rather than
manufacturing one — but the honest reading is that the test was inconclusive by
construction, not that the two profiles are equivalent. A benchmark that cannot
detect a difference is not evidence of absence.

### 4.3 A second run that measured something

Repeated with presentation unlocked (`MESA_VK_WSI_PRESENT_MODE=immediate`), so
the workload became CPU-bound on the driver:

```
A_run1 (power-saver)   73158 frames  avg 6096.8 fps  1% low 1984.9 fps  p99 0.40 ms
A_run2 (power-saver)   72727 frames  avg 6060.9 fps  1% low 1943.7 fps  p99 0.41 ms
B_run1 (performance)   86345 frames  avg 7195.7 fps  1% low 2146.4 fps  p99 0.37 ms
B_run2 (performance)   88276 frames  avg 7356.8 fps  1% low 2243.0 fps  p99 0.35 ms

measured noise floor from 2 baseline runs: 2.1%

A (baseline) vs B (candidate):
  1% low: 1984.9 fps → 2146.4 fps
  P99 frametime: 0.4 ms → 0.4 ms
  P95 frametime: 0.2 ms → 0.2 ms
  Average FPS: 6096.8 fps → 7195.7 fps
```

All four metrics moved beyond the 2.1% floor, so all four are reported as
improvements. The pipeline works: capture, parse, statistics, measured noise
floor, direction-aware comparison.

### 4.4 What this does and does not show

**It shows** that the engine produces a defensible answer from real data, that
the noise floor suppresses small differences, and that on this machine the
`performance` power profile measurably beats `power-saver` for a CPU-bound
Vulkan workload — which is the direction Booster moves it.

**It does not show** anything about games. vkcube at 7000 FPS and 0.13 ms
frametimes is a degenerate workload; those absolute numbers are meaningless as a
gaming proxy, and the 15% figure must not be restated as "Booster gives 15% more
FPS". That is precisely the extrapolation the brief forbids, and the reason the
application reports nothing until a game has actually been measured.

## 5. What is still missing

1. **Per-game capture wired into the Booster flow.** The engine exists; nothing
   calls it during an activation.
2. **Alternating runs.** A-B-A-B rather than AA-BB, so thermal drift does not
   land entirely on one arm. The validation above used AA-BB.
3. **Discarding the first run** for shader compilation and cache warming.
4. **Five runs per arm, not two.** Two sets a floor; it does not characterise a
   distribution.
5. **A game with a repeatable benchmark.** The two titles installed here are
   online and anti-cheat protected, and launching someone's competitive games
   repeatedly on their own account was not a reasonable thing to do unasked.

Until 1–5 hold, the report says "not measured", and per the brief that is the
point.
