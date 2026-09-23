# 03 — Gaming Distribution Research

What the modern Linux gaming distributions actually do, and which of it applies
to BigLinux.

**Research conducted 2026-09-23.** Sources are listed per section. Where a claim
could be checked against the software installed on the reference machine, it
was, and the check is shown — upstream documentation and the binary on disk do
not always agree.

---

## 1. PikaOS / falcond

falcond is the daemon BiGame-mode already builds on, so its behaviour is the
single most load-bearing external fact in this project.

### 1.1 What it controls

From the upstream README and the falcond wiki:

| Field | Effect |
|---|---|
| `enable_performance_mode` | master switch |
| `scx_sched` | scheduler to load — the documented set is far wider than the five this project had hardcoded |
| `scx_sched_props` | `default`, `gaming`, `power`, `latency`, `server` |
| `vcache_mode` | `none`, `cache`, `freq` |
| `profile_mode` | `none`, `handheld`, `htpc` |
| `poll_interval_ms` | `/proc` scan interval, default 9000 |

Per-profile it adds `start_script`, `stop_script`, `idle_inhibit`, and — in
newer releases — `dmem_protect` and `disable_split_lock`.

### 1.2 Checked against the installed build

| Claim | Result |
|---|---|
| Config is `/etc/falcond/config.conf` | **Confirmed.** `strings` finds that path and no other; `/etc/falcond/` contains exactly that file. This is what proved audit finding CFG-01. |
| Profiles live in `/usr/share/falcond/profiles/`, user overrides in `user/` | Confirmed on disk |
| Matches games by process name | Confirmed — the binary contains `/proc/%i/comm`, `/proc/%i/exe` and `matched pid= name='' profile='` |
| Manages the power profile | Confirmed — binary references `org.freedesktop.UPower.PowerProfiles`; status file carries `RESTORE_STATE: Power Profile:` |
| `dmem_protect` / `disable_split_lock` | **Not in 2.0.2.** No such strings in the installed binary; they are newer additions |
| Runs as root | Confirmed — `User=root`, and it spawns `start_script` through `/bin/sh` |

The last two rows changed the implementation. Newer fields this build does not
know are now preserved verbatim on save rather than silently dropped, and script
hooks are refused by the privileged helper, because "save a game profile" must
not be a way to run code as root.

### 1.3 What was taken

* falcond owns the scheduler, V-Cache and per-game activation. BiGame-mode
  configures it; it does not duplicate it.
* Profile keys are process names. This is the fix for GAME-01.
* `handheld` / `htpc` profile modes are a falcond concept, not something to
  reimplement. Worth noting that the reference machine — a desktop — was left in
  `profile_mode = handheld` by an earlier run, which is a misconfiguration this
  project should surface rather than perpetuate.

### 1.4 What was left alone

DMEM cgroup protection and split-lock mitigation are falcond's to apply per
profile. Adding a parallel implementation would put a second writer on state
falcond already manages, which is the exact failure [02](02-PERFORMANCE-AUTHORITY.md)
exists to prevent.

Sources: [falcond README](https://github.com/PikaOS-Linux/falcond/blob/main/README.md),
[PikaOS wiki — falcond auto-gamemode](https://wiki.pika-os.com/en/custom-utils-wiki/pikaos-falcond-auto-gamemode),
[falcond releases](https://git.pika-os.com/general-packages/falcond/releases),
[falcond-profiles](https://github.com/PikaOS-Linux/falcond-profiles).

---

## 2. CachyOS

### 2.1 How schedulers are actually managed

CachyOS's sched-ext story is a service plus a CLI, not a set of unit files:

* `scx_loader` is a D-Bus service that starts a scheduler with a named profile
  and persists the choice in `/etc/scx_loader.toml`.
* `scxctl` is the client: `scxctl start --sched bpfland --mode gaming`.
* Profiles are flag presets per scheduler — `scx_bpfland` has a `gaming_mode`
  entry, `scx_lavd` maps its gaming mode to `--performance`.

Notably, **`scx_loader` ships without a default scheduler configured**. That
matches what falcond's shipped profiles do — they all set `scx_sched = none` —
and it matches the reference machine, where sched-ext is available and idle.

### 2.2 On scheduler choice

The documented picture is that `scx_lavd` (latency-criticality aware virtual
deadline) targets gaming directly and reports gains in 1% lows and average FPS,
that `scx_bpfland` is strong under heavy background load, and that CachyOS's
handheld edition defaults to LAVD.

**This was not adopted as a hardcoded mapping.** The brief is explicit that
`bpfland = gaming` must not be frozen in, and the reference machine makes the
reason concrete: it has sixteen schedulers installed, including several
(`beerland`, `cake`, `cosmos`, `flow`, `forge`, `mlfq`, `pandemonium`,
`tickless`) that did not exist when the old five-variant enum was written. The
code enumerates what is installed instead.

### 2.3 What was taken

* Capability detection as a first-class concept: kernel support, installed
  schedulers, loader service, and CLI are four separate questions with four
  separate answers.
* The distinction between "no sched-ext" and "sched-ext but no loader", which
  leads to different advice.

### 2.4 What was not taken

CachyOS's kernel patches and its own kernel manager. BigLinux has its own kernel
policy — the reference machine runs XanMod — and an application is the wrong
layer to be swapping kernels from.

Sources: [CachyOS sched-ext tutorial](https://wiki.cachyos.org/configuration/sched-ext/),
[CachyOS kernel manager](https://wiki.cachyos.org/features/kernel_manager/),
[Deploying and managing sched_ext schedulers in CachyOS (LPC)](https://lpc.events/event/18/contributions/1873/attachments/1417/3036/sched-ext%20CachyOS.pdf).

---

## 3. Bazzite

The most directly useful finding in this whole review, because it is a decision
rather than a feature.

**Bazzite removed Feral GameMode from its desktop images.** The stated reasoning
is that its function is covered by components that require no per-game setup —
the System76 scheduler and joystickawake — and that its inclusion had been an
oversight. Users who had `gamemoderun %command%` in their launch options found
games failing to start, and the resolution was to delete the launch option
rather than reinstate the package.

Two things carry over:

1. **Stacking optimization layers is a cost, not a feature.** Each additional
   daemon that snapshots and restores the same state is another chance to
   restore the wrong baseline.
2. **Per-game launch options are a bad interface.** A daemon that notices the
   game is strictly better than one the user must remember to invoke, which is
   also the argument for falcond's `/proc` scanning over GameMode's opt-in.

Both fed directly into [02](02-PERFORMANCE-AUTHORITY.md).

What was not taken: Bazzite's image-based delivery and its Gamescope session.
BigLinux is a traditional package-managed KDE desktop; a full Gamescope session
is a different product decision, not an application feature.

Sources: [ublue-os/bazzite#777](https://github.com/ublue-os/bazzite/issues/777),
[Bazzite Desktop removed Feral GameMode — discussion](https://social.dn42.us/post/258914),
[Bazzite support thread](https://www.answeroverflow.com/m/1337307971733946398).

---

## 4. Nobara

Nobara ships falcond too, and documents it in its own wiki — useful
corroboration that falcond's interface is stable enough to build on and is not
specific to PikaOS.

Nobara's other work is largely kernel patches, Gamescope patches, and packaging
of Wine/Proton builds. None of that belongs in an application: patching
Gamescope is the distribution's job, and an application that assumed a patched
Gamescope would break on a stock one. That assumption is exactly what the
capability probing in `capabilities.rs` refuses to make.

Source: [Nobara wiki — falcond](https://wiki.nobaraproject.org/general-usage/additional-software/falcond).

---

## 5. What none of them do, and this project now does

None of the projects reviewed verifies that its own changes took effect, or
separates "applied" from "improved" when reporting to the user. They apply
settings believed to be good and move on — which is reasonable for a daemon with
no UI, and insufficient for an application whose entire screen says "6
optimizations active".

That gap is the one genuinely original part of this design, and it is why
`Knob::verify` is mandatory rather than optional, and why
`Outcome::NotMeasured` is a value rather than an error.

---

## 6. Summary

| Idea | Source | Adopted? |
|---|---|---|
| falcond as the game-scoped authority | PikaOS | Yes |
| Profile keys are process names | falcond behaviour | Yes — fixes GAME-01 |
| Config at `/etc/falcond/config.conf` | falcond binary | Yes — fixes CFG-01 |
| Preserve unknown profile fields | falcond release history | Yes |
| Capability detection for sched-ext | CachyOS | Yes |
| `scx_loader` / `scxctl` as the switching mechanism | CachyOS | Detected, not reimplemented |
| Fixed `bpfland = gaming` mapping | CachyOS docs | **No** — enumerate instead |
| Do not stack GameMode on another layer | Bazzite | Yes |
| No per-game launch options required | Bazzite | Yes |
| Kernel patching, custom kernels | CachyOS, Nobara | No — wrong layer |
| Gamescope session as the desktop | Bazzite, SteamOS | No — different product |
| Verify every change; separate applied from improved | — | New here |
