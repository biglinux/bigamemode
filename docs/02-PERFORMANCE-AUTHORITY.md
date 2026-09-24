# 02 — Performance Authority

Which component owns which piece of system state, and why.

The rule: **every resource has exactly one writer.** Where two components can
write the same thing, one of them stands down — explicitly, visibly, and with
the reason shown to the user. A setting quietly overwritten by another daemon is
worse than a setting never applied, because the first reports success.

---

## 1. The authority table

| Resource | Owner | BiGame-mode's role |
|---|---|---|
| sched-ext scheduler | **falcond** | never writes it; reports why |
| Per-game profile activation | **falcond** | writes profile files, never activates |
| 3D V-Cache mode (during a game) | **falcond** | via profile |
| 3D V-Cache mode (globally) | **Booster** | only when no game profile is active |
| Power profile (during a game) | **falcond** | Booster stands down |
| Power profile (otherwise) | **Booster** | snapshot → apply → verify → restore |
| CPU governor / EPP | **Booster** | via the privileged helper |
| GPU DPM level | **Booster** | render GPU only |
| Gamescope invocation | **Launch pipeline** | built once, capability-gated |
| Frame generation | **Launch pipeline** | one backend at a time |
| Network measurement | **BiGame-mode** | measures; does not tune |
| Telemetry | **BiGame-mode** | reads only |

---

## 2. falcond versus Feral GameMode

### 2.1 What was measured

GameMode is **not installed** on the reference machine — neither `gamemoded`
nor `gamemoderun` exists, and there is no user unit. falcond 2.0.2 is installed,
enabled and running. So the two could not be observed contending here, and the
analysis below is from the two projects' documented behaviour plus what the
falcond binary on disk actually does.

### 2.2 Where they overlap

Both projects exist to do the same job — notice a game, raise the machine's
performance posture, and put it back afterwards — and they overlap on:

| Area | GameMode | falcond |
|---|---|---|
| CPU governor | sets `performance` | per-profile `cpu_governor` |
| Power profile | switches via PPD | switches via `org.freedesktop.UPower.PowerProfiles` |
| Process priority / nice | yes | — |
| I/O priority | yes | — |
| GPU performance level | optional, opt-in | — |
| sched-ext scheduler | no | **yes** |
| 3D V-Cache mode | no | **yes** |
| Screensaver inhibit | yes | `idle_inhibit` |
| Start/stop scripts | yes | `start_script` / `stop_script` |
| Activation | game must opt in (`gamemoderun`, or the library) | automatic, by scanning `/proc` |

The critical difference is the last row. GameMode is **request-based**: a game
asks for it, by being launched through `gamemoderun` or by linking
`libgamemode`. falcond is **observation-based**: it scans `/proc` on an interval
and matches process names against profiles.

### 2.3 The decision

**falcond is the performance authority for game-scoped state. GameMode is not
installed, not recommended alongside it, and not integrated.**

Three reasons, in order of weight:

1. **Both snapshot and restore the same state, independently.** Each records
   "what the governor was before" and puts it back when its own idea of the
   game ends. If both are active and their notions of "the game" differ by even
   a second, the second one to restore writes the first one's *boosted* value
   back as if it were the baseline. The machine then stays boosted after the
   game exits, and nothing reports an error, because from each daemon's own
   point of view it did exactly what it was told.

2. **falcond does strictly more, in the areas that matter most here.** Scheduler
   selection and V-Cache mode are the two levers with the best evidence behind
   them on modern AMD hardware, and GameMode does neither. The things GameMode
   adds over falcond — `nice` and I/O priority — are the weakest items in its
   own list; a game thread that is losing to another process is better addressed
   by the scheduler, which is precisely falcond's domain.

3. **This is the direction the ecosystem has taken.** Bazzite removed Feral
   GameMode from its desktop images, on the grounds that its function is covered
   by components that need no per-game launch option. That reasoning applies
   here unchanged: asking a user to add `gamemoderun %command%` to every game is
   a worse experience than a daemon that notices the game by itself.

**If a user does have GameMode installed**, the conflict engine reports it
(`Capabilities::gamemode`) rather than silently competing. It is surfaced as a
configuration warning, not auto-removed — removing software the user installed
is not this application's decision to make.

---

## 3. falcond versus Booster Mode: the power profile

This one *is* a live conflict, and it was found by reading falcond rather than
by reasoning about it.

### 3.1 Evidence

```
$ strings /usr/sbin/falcond | grep -i powerprofiles
/org/freedesktop/UPower/PowerProfiles
org.freedesktop.UPower.PowerProfiles
warning(power_profiles): Failed to load initial state:
```

and falcond's status file carries a restore record:

```
RESTORE_STATE:
  SCX Scheduler: none (Mode: default)
  Power Profile: balanced
```

So falcond captures the power profile when a game matches, changes it, and
writes its captured value back when the game exits.

### 3.2 The failure this creates

Booster writes `performance` while a game is running. falcond's captured
baseline still says `balanced` — it was taken before Booster ran. The game
exits, falcond restores `balanced`, and Booster's change is gone. Booster's own
verification had already passed, because at the instant it read the knob back,
the value *was* correct.

That is the worst shape a bug can take in this project: a true report that
becomes false later, with nothing to notice.

### 3.3 The resolution

`plan::power_profile_owner()` asks falcond, through its status file, whether it
currently holds a profile:

```rust
match crate::status::read().and_then(|s| s.active_profile) {
    Some(profile) => PowerProfileOwner::Falcond { profile },
    None          => PowerProfileOwner::Booster,
}
```

While falcond holds one, Booster does not plan the power profile at all, and the
report says so in as many words:

> Power profile — falcond is managing the power profile for 'Cyberpunk2077.exe'
> and will restore it when the game exits

Everything falcond does not manage is still planned normally. Covered by
`booster_stands_down_while_falcond_holds_a_profile` and
`booster_owns_the_power_profile_when_no_game_is_active`.

---

## 4. Why Booster never writes the scheduler

falcond owns sched-ext, and the Booster's planner records that as a decision
rather than an omission:

> sched-ext scheduler — falcond owns the scheduler; Booster does not write it to
> avoid two controllers contending for the same state

This holds even when the scheduler *is* switchable. It would be easy to write
`scx_lavd` from the Booster and see an immediate effect; it would also mean that
the next time falcond activated a profile with `scx_sched = none`, the scheduler
would be torn down under a running game.

The correct place to express "use a gaming scheduler" is falcond's own
configuration, which BiGame-mode writes through the Tuning page — to
`/etc/falcond/config.conf`, the file falcond actually reads. That is a
configuration change with one writer, not a runtime race with two.

### 4.1 On this machine it is moot, and that is worth saying

sched-ext is compiled into the kernel and sixteen `scx_*` schedulers are
installed, but the D-Bus service falcond delegates to does not exist:

```
falcond[5569]: warning(scx_loader): Failed to load initial state: ServiceUnknown
$ cat /sys/kernel/sched_ext/state
disabled
$ which scxctl     # not installed
```

`SchedExtCaps::switchable()` reports this as
`ServiceDown("scx_loader service is not running")`, which is a different
statement from "your hardware cannot do this" and leads to a different fix
(install `scx_loader`, or `scxctl`). The previous UI offered a scheduler picker
here with no indication that every selection would be discarded.

---

## 5. Gamescope and Tuning are complementary

Answering the brief's question directly: **they are complementary, not mutually
exclusive**, and the architecture treats them as operating on different layers.

```
Presentation layer   Gamescope, FSR/NIS, frame generation, VRR, HDR, FPS cap
        ↑                     owned by the launch pipeline
        │
Execution layer      scheduler, V-Cache, governor, EPP, GPU DPM, power profile
                              owned by falcond and Booster
```

Nothing in the lower group changes what Gamescope does, and nothing Gamescope
does changes what the scheduler does. The genuine conflicts are *within* the
presentation layer, where several components implement the same function — and
those are enumerated in [06-GAMESCOPE-TUNING.md](06-GAMESCOPE-TUNING.md).

---

## 6. Consequences for the code

* `Knob::privilege()` names, per knob, who is allowed to write it.
* `Plan::build_with_owner()` takes the power-profile owner as an argument, so
  the arbitration is testable without a running falcond.
* `Skipped` carries a reason for every candidate not planned, and the report
  shows them — which is how a user can tell the engine reasoned rather than
  guessed.
* The privileged helper writes `/etc/falcond/config.conf` and asks systemd to
  reload falcond. It never activates a profile itself; that stays falcond's job.
