# 09 — Benchmarks

## 1. Status: not built

**No benchmark engine was implemented in this pass, and no performance
measurements were taken.** This document exists to say that plainly, to record
why, and to specify what building one requires — not to stand in for work that
was not done.

Every report the application produces therefore says:

```
Performance impact not measured
```

That is the correct output, not a placeholder. It is also the single most
important behaviour in the project: a report with six verified changes and no
benchmark still says exactly that.

## 2. Why nothing was measured

The reference machine could not produce a defensible A/B comparison in this
pass, for reasons that are worth stating rather than glossing:

1. **The machine is already at its ceiling.** Power profile `performance`, CPU
   governor `performance`, EPP `performance`. Booster's plan on this machine at
   rest is a single change — GPU DPM `auto` → `high`. Benchmarking a one-knob
   delta needs far more care than a broad before/after.
2. **The two most interesting knobs could not be applied.** `bigame-daemon` is
   not installed here, so CPU governor and GPU DPM writes fail with
   `ServiceUnknown`. Measuring the effect of a change that did not happen is not
   a benchmark.
3. **Running a benchmark means running a game.** The installed titles are ARC
   Raiders and Dead by Daylight — both online, both anti-cheat protected, and
   neither with a repeatable built-in benchmark. Launching someone's online
   competitive games repeatedly on their own account, on their machine, is not a
   reasonable thing to do unasked.
4. **A synthetic benchmark would not answer the question.** The brief is
   explicit that one synthetic run must not be used to conclude that all games
   improved. A `vkmark` score would be a number, and it would be a number about
   `vkmark`.

Measuring badly and reporting the result would have been worse than reporting
nothing. The architecture is built so that reporting nothing is a first-class
outcome rather than a gap.

## 3. What does exist

`Outcome` is in place and fully wired, so the day a measurement exists there is
somewhere honest for it to go:

```rust
pub enum Outcome {
    NotMeasured,
    Improved  { metric, before, after, unit },
    NoChange  { metric },
    Regressed { metric, before, after, unit },
}
```

`Regressed` is not decoration. An engine that can only report improvements is an
engine that will eventually report one that did not happen.

`Report::measurements` is a `Vec<Outcome>`; empty means nothing was benchmarked,
and `performance_claim()` returns the "not measured" line. The UI's Performance
group renders that with an explanation rather than hiding it.

Also already available and directly reusable:

* `network::LatencyStats` — median, p95 and jitter with the ordering
  subtleties already handled, and `from_samples` returning `None` for zero
  samples rather than summarising nothing as zero.
* `hardware::Gpu::hwmon_u64` and `busy_percent` — temperature, clock, power draw
  and utilisation from the **render** GPU, which is the fix described in
  [07](07-NETWORK-TELEMETRY.md).
* `Snapshot` — a captured baseline is exactly what an A/B run needs on one side.

## 4. What building it requires

### 4.1 Capture

MangoHud already writes CSV logs and is installed here (0.8.4). `MANGOHUD_CONFIG`
with `output_folder`, `log_duration` and `autostart_log` produces per-frame
frametimes without any new dependency and without modifying the game.

Frametime is the primary series. FPS is derived from it, never captured
directly: an averaged FPS counter destroys exactly the information that matters.

### 4.2 Metrics

From the frametime series: mean, P95, P99, 1% low, 0.1% low, and a stutter count
(frames exceeding some multiple of the running median). Alongside it, sampled at
a fixed interval: CPU and GPU utilisation and clocks, temperatures, power draw,
VRAM and RAM.

1% low is reported in preference to average FPS. It is what players perceive as
smoothness, and it is the metric a scheduler change is most likely to move.

### 4.3 Method

The hard part is not collection, it is making the comparison mean something:

* **Alternate, do not batch.** A-B-A-B, not AAA-BBB, so thermal drift and
  background load do not land entirely on one arm.
* **Discard the first run.** Shader compilation and cache warming make it
  unrepresentative.
* **Repeat enough to see the noise.** At least five runs per arm, and report the
  spread, not just the centre.
* **Report a difference only when it exceeds the measured noise floor.** If the
  A-to-A variation is 4%, a 3% A-to-B difference is `NoChange`.
* **Same scene, same settings, same duration.** A benchmark that cannot be
  repeated is an anecdote.

### 4.4 Scope

Per-game, and labelled as such. "This machine got faster in this game" is a
defensible claim. "Booster improves performance" is not one a single title can
support, and the report must never round the first up to the second.

## 5. What would have to be true to ship it

A game with a repeatable built-in benchmark, on hardware where the changes can
actually be applied, with the user's explicit agreement to run their games
repeatedly. None of the three held here.

Until then the application tells the truth about what it did and stays silent
about what it did not measure — which, per the brief, is the point.
