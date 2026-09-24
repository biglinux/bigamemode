# 05 — Booster Architecture

## 1. What it replaced

The entire previous implementation, in full:

```rust
let profile = if row.is_active() { "performance" } else { "balanced" };
gio::spawn_blocking(move || { let _ = bigame_core::dbus::power_profile_set(&target); });
```

An `AdwSwitchRow`, one D-Bus property write, and a discarded return value. "Off"
wrote the literal string `balanced`, so on a machine resting in `performance` —
which the reference machine is — a single toggle cycle permanently degraded the
baseline. A failed write still flipped the switch and still turned the row green.

Three properties were missing, and all three are structural rather than
cosmetic: it did not record what it was changing from, it did not check whether
the change happened, and it could not put anything back.

## 2. The pipeline

```
Detect → Snapshot → Plan → Apply → Verify → Report → (later) Restore
```

Every stage is mandatory. Two of them are the point of the whole design:

* **Snapshot** runs before anything is written, so "off" returns the machine to
  the state it was actually in.
* **Verify** runs after every write, so the report describes what the system
  did rather than what it was asked to do.

```
BoosterEngine
├── hardware::Hardware      what this machine is
├── capabilities::Capabilities   what it can be asked to do
├── booster::knob::Knob      one piece of state: read / write / verify
├── booster::snapshot        capture, and restore exactly
├── booster::plan            decide — and be willing to decide nothing
├── booster::journal         atomic, boot-aware crash recovery
└── booster::report          applied ≠ improved
```

## 3. Knob

A knob is the smallest unit the engine understands, and it is deliberately
narrow: report the current value, write a new one, say who may write it.

```rust
pub enum Knob {
    PowerProfile,
    CpuGovernor,
    CpuEpp,
    GpuDpmLevel { card: String },
    VCacheMode,
}
```

Three properties matter.

**Reads never hit a cache.** Verification is worthless if it consults the value
we just wrote from memory.

**`allowed_values()` comes from the machine**, not a constant. On this bench
`amd-pstate-epp` accepts only `performance` and `powersave`, so a plan
mentioning `schedutil` is rejected before the privileged helper is ever called.

**Writing and verifying are separate calls.** `write()` returning `Ok` means the
request was accepted, nothing more:

```rust
pub enum Verification {
    Confirmed,
    Mismatch { actual: String },  // a second writer is contending
    Unreadable,
}
```

`Mismatch` is reported distinctly in the UI, because a write that was accepted
but did not stick is a configuration problem — another daemon owns that knob —
and not a transient failure.

## 4. Snapshot and rollback

```rust
Snapshot::capture(&knobs)          // before anything is written
snapshot.restore_applied(&applied) // only what was actually changed, in reverse
```

Two rules, both learned from a live run rather than from theory.

**Only restore what was applied.** A snapshot captures broadly, because a wide
baseline makes a better report; rollback must be narrow. An early run captured
`cpu_epp` as `power` while the machine sat in `power-saver`, never planned it,
and then tried to write `power` back during rollback — which the driver refused,
because with the governor at `performance` the only accepted EPP is
`performance`. The knob reached the right value a moment later anyway, when the
power profile it depends on was restored.

**Restore in reverse order.** Knobs depend on each other; a power profile drives
the governor and the EPP. The last thing changed is the first thing put back.

A knob whose baseline could not be *read* is never *written*: `Plan` refuses to
plan it and records `Skipped::NotRestorable`. An unrestorable change is not worth
making.

## 5. Plan

The planner is the only component allowed to have an opinion, and it must answer
five questions per candidate — what it alters, why that can help, what hardware
it needs, how support was detected, how it is undone. A candidate that cannot
answer all five is not planned.

It is equally important that it can plan **nothing**:

```
--- PLAN (1 changes) ---
  gpu_dpm_level:card1 : auto -> high  [Thermal]
      Keeps card1 at its high DPM state so the first frames after a load
      screen are not rendered at idle clocks.
--- SKIPPED (4) ---
  AlreadyOptimal { knob: "Power profile", value: "performance" }
  AlreadyOptimal { knob: "CPU governor", value: "performance" }
  Unsupported { knob: "3D V-Cache mode", detail: "this CPU has no 3D V-Cache" }
  Unsupported { knob: "sched-ext scheduler", detail: "scx_loader service is not running" }
```

That is the real output on the reference machine at rest. Reporting the rejected
candidates with reasons is what lets a user tell that the engine reasoned rather
than guessed — and it is the difference between "already optimal" and "did
nothing".

Skip reasons are typed, not strings:

```rust
enum Skipped {
    Unsupported { knob, detail },     // hardware or software cannot do it
    AlreadyOptimal { knob, value },   // already holds the target
    NotBeneficial { knob, detail },   // would cost more than it gains
    NotRestorable { knob },           // no baseline — must not be touched
}
```

### 5.1 Battery

On battery the automatic plan raises nothing. Pinning clocks high on a laptop
usually costs more in thermal throttling and ceiling than it returns, and the
user did not ask to spend their battery. Covered by
`refuses_to_raise_power_draw_on_battery`.

`NOT TESTED — hardware unavailable`: the reference machine is a desktop with no
battery, so this path is unit-tested only.

### 5.2 Only the render GPU

`consider_gpu_dpm` targets `hardware.render_gpu()`. Pinning an idle integrated
GPU to `high` spends power for nothing — and on this dual-GPU bench, "the first
card sysfs yields" *is* the idle integrated one.

## 6. Journal

Booster changes state that outlives the process. If the UI is killed mid-apply,
something has to know what the machine looked like beforehand.

`$XDG_STATE_HOME/bigame-mode/booster-journal.json`, `0600`, written to a temp
file in the same directory and `rename(2)`d into place after `fsync`, so a
reader never sees a half-written record and a power cut cannot leave the name
pointing at unflushed data.

Every record carries the kernel's boot id. sysfs knobs reset themselves at boot,
so a journal from a previous boot describes a state the machine is no longer in;
it is discarded rather than replayed. A corrupt or version-mismatched record is
removed so it cannot keep failing on every start.

The journal is written **before** the first change. If it cannot be written, the
run stops and nothing is touched — a change we could not undo is the one thing
this engine will not make.

## 7. Report

```rust
pub enum Outcome {
    NotMeasured,                                  // the honest default
    Improved  { metric, before, after, unit },
    NoChange  { metric },
    Regressed { metric, before, after, unit },
}
```

`NotMeasured` is a value, not an error. Writing `performance` to a knob and
reading it back proves the system changed; it does not prove a frame got faster.
With no benchmark, the report says:

> Performance impact not measured

and the UI explains why in one line. A report with six verified changes and no
measurement still says that.

## 8. Verified on the reference machine

From `power-saver`, with `bigame-daemon` deliberately not installed so the
failure path is exercised:

```
$ booster_run plan
  power_profile       power-saver -> performance   [Safe]
  cpu_governor        powersave   -> performance   [Safe]
  gpu_dpm_level:card1 auto        -> high          [Thermal]

$ booster_run on
  [1/3] applying Power profile
           verifying Power profile
  [2/3] applying CPU governor      → ServiceUnknown
  [3/3] applying GPU power level   → ServiceUnknown

  1 of 3 optimizations verified
  Power profile: power-saver → performance   Confirmed
  CPU governor: powersave → performance      error: ServiceUnknown
  GPU power level (card1): auto → high       error: ServiceUnknown
  performance: Performance impact not measured

$ booster_run off
  restore power_profile -> power-saver : Restored
  active after: None            # journal cleared
```

Four things to note. The two root-requiring knobs failed **loudly**. The report
said "1 of 3", not "3 applied". Only the knob that was actually applied was
restored. And the restore target was `power-saver` — the value that was really
there — where the old implementation would have written `balanced`.

## 9. Threading

The engine is `async` and every privileged call goes through async zbus. The GTK
main loop is not a Tokio reactor, so the UI runs the engine on a dedicated
thread with its own current-thread runtime and receives `Progress` events over a
channel.

This surfaced a bug worth recording. zbus's *blocking* API drives its own
executor, and Tokio panics outright if that happens on a runtime worker —
*"Cannot start a runtime from within a runtime"*. Pressing Booster would have
crashed the UI. `dbus::blocking_dbus()` now detects whether a runtime is running
and, if so, moves the call to a plain OS thread:

```rust
if tokio::runtime::Handle::try_current().is_err() {
    return Some(f());          // no runtime — call directly
}
std::thread::Builder::new().spawn(f).ok()?.join().ok()
```

It was found by building a small CLI harness over the same engine and running
it, which is a good argument for having one.

## 10. Extending it

Adding an optimization means adding a `Knob` variant and a `consider_*` method.
The variant supplies read, write, verify and privilege; the method supplies the
decision and the rationale. Snapshot, journal, rollback and reporting then apply
automatically — which is the property that makes "did this actually take effect"
a fact about the architecture rather than something each feature must remember.
