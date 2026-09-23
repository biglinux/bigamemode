# 01 — Audit

Systematic audit of the code at branch point `7f98d40`, cross-checked against
the real system described in [00-BASELINE.md](00-BASELINE.md).

Every finding below was verified against the running machine, the installed
`falcond` 2.0.2 binary, or a reproducible proof-of-concept. Findings that could
only be reasoned about are labelled **ANALYSIS**; everything else is
**VERIFIED**.

Severity: **S0** exploitable / data-destroying · **S1** feature does not work at
all · **S2** works but wrong or misleading · **S3** quality.

---

## 0. Summary table

| ID | Area | Finding | Sev | Action |
|---|---|---|---|---|
| SEC-01 | daemon | Root D-Bus service performs **no authorization at all** | S0 | REWRITE |
| SEC-02 | daemon | `save_profile` path traversal → arbitrary root file write | S0 | REWRITE |
| SEC-03 | daemon | `delete_profile` path traversal → arbitrary root file delete | S0 | REWRITE |
| SEC-04 | packaging | `sudoers` NOPASSWD wildcard → passwordless root for `wheel` | S0 | REMOVE |
| SEC-05 | UI | "Repair & Enable" deletes **all** user profiles via `pkexec` | S0 | REWRITE |
| SEC-06 | daemon | `pkill -HUP falcond` signals by name | S2 | IMPROVE |
| SEC-07 | policy | `.policy` actions defined but never referenced | S1 | REWRITE |
| CFG-01 | core | falcond config path is wrong → whole global config is a no-op | S1 | REWRITE |
| CFG-02 | core/daemon | V-Cache sysfs path differs between core and daemon; both wrong | S1 | REWRITE |
| BST-01 | UI | Booster Mode == `powerprofilesctl set performance` | S1 | REWRITE |
| BST-02 | UI | Booster "off" hardcodes `balanced`, destroying prior state | S2 | REWRITE |
| BST-03 | — | No snapshot, no verification, no rollback, no journal | S1 | REWRITE |
| GS-01 | core | `gamescope::Config::to_args()` emits `--fsr`, rejected by 3.16 | S1 | REWRITE |
| GS-02 | core | No gamescope version/capability detection | S2 | IMPROVE |
| GS-03 | core | Two unconnected gamescope models | S3 | MERGE |
| GS-04 | core | Nested gamescope never gets `-f`/`-b` | S2 | IMPROVE |
| TEL-01 | core | GPU telemetry aborts on first non-GPU DRM node | S1 | REWRITE |
| TEL-02 | core | GPU telemetry picks the iGPU on dual-GPU systems | S2 | REWRITE |
| GAME-01 | core | Steam detection stores `installdir`, which never matches a process | S1 | REWRITE |
| GAME-02 | core | No process-tree / Proton-aware detection | S2 | IMPROVE |
| SCX-01 | core | Hardcoded scheduler enum; 16 installed, 5 known | S2 | REWRITE |
| SCX-02 | core | No detection that `scx_loader` is absent | S2 | IMPROVE |
| LNCH-01 | core | Video pipeline gated on PowerProfiles == `performance` | S2 | REWRITE |
| LNCH-02 | core | Steam `-applaunch` bypasses the entire pipeline | S2 | IMPROVE |
| PROF-01 | core | Saving a profile applies its CPU governor **globally**, forever | S2 | REWRITE |
| PROF-02 | core | Saving a profile never tells falcond to reload | S1 | IMPROVE |
| PROF-03 | core | `delete()` shells out to `sudo -n pkill` | S2 | REWRITE |
| NET-01 | — | No network or DNS feature exists at all | S1 | (new) |
| DBUS-01 | core | Session status service polls a file every 500 ms forever | S3 | IMPROVE |
| T-01 | tests | Test results depend on the host's live power profile | S2 | IMPROVE |
| FMT-01 | repo | `cargo fmt --check` fails | S3 | IMPROVE |
| REPO-01 | repo | Build artifacts and a nested repo copy committed | S3 | REMOVE |

---

## 1. Security — the daemon is a local root escalation

### SEC-01 · No authorization · **VERIFIED** · S0

`bigame-daemon` owns `com.biglinux.BiGameMode` on the **system** bus and exposes
`save_profile`, `delete_profile`, `apply_falcond_config`, `set_vcache_mode`,
`set_cpu_governor`. Not one of them checks the caller.

The shipped bus policy (`data/com.biglinux.BiGameMode.conf`) grants access to
everybody:

```xml
<policy context="default">
  <allow send_destination="com.biglinux.BiGameMode"/>
</policy>
```

`bigame-core/src/polkit.rs` is the entire authorization layer:

```rust
pub mod actions {
    pub const SET_GOVERNOR: &str = "com.biglinux.bigamemode.set-governor";
    …
}
```

Five string constants. `grep -r` finds no use of any of them, and the daemon
never links Polkit. `data/com.biglinux.BiGameMode.policy` declares five actions
with `auth_admin` — all decoration (**SEC-07**).

Net effect: any local uid — a sandboxed app, a compromised browser helper, a
container with system-bus access — can drive privileged system state.

### SEC-02 / SEC-03 · Path traversal → arbitrary root write · **VERIFIED** · S0

```rust
let file_path = target_dir.join(format!("{}.conf", name));
fs::write(&file_path, json_payload)
```

`name` comes straight off the bus and is never validated. Reproduced with the
exact `Path::join` semantics the daemon uses:

```
name "../../../../../etc/cron.d/pwn"
  -> writes /etc/cron.d/pwn.conf
name "../../../../../etc/systemd/system/pwn.service"
  -> writes /etc/systemd/system/pwn.service.conf
```

Combined with SEC-01 this is **unauthenticated local privilege escalation to
root**, with fully attacker-controlled file contents. `delete_profile` has the
identical flaw for `fs::remove_file`.

Note the UI-side `profiles::critical_errors()` does reject `..` — but it runs in
the *client*, which an attacker simply does not use. Validation must be in the
daemon.

### SEC-04 · sudoers wildcard → passwordless root · **VERIFIED** · S0

`etc/sudoers.d/bigame-mode` ships:

```
Cmnd_Alias BIGAME_WRITE = /usr/bin/tee /etc/falcond/config.conf, \
                          /usr/bin/tee /usr/share/falcond/profiles/user/*
%wheel ALL=(root) NOPASSWD: BIGAME_DIRS, BIGAME_WRITE, BIGAME_DELETE, BIGAME_SYSFS, BIGAME_DAEMON
```

`sudoers(5)` is explicit about what `*` means in an **argument**:

> A forward slash (‘/’) will not be matched by wildcards used in the file name
> portion of the command. **When matching the command line arguments, however, a
> slash does get matched by wildcards** since command line arguments may contain
> arbitrary strings and not just path names.

So `sudo tee /usr/share/falcond/profiles/user/../../../../../etc/sudoers`
matches the rule and runs passwordless as root. `BIGAME_SYSFS` and
`BIGAME_DELETE` are the same shape. A second, overlapping sudoers file
(`data/bigame-mode-sudoers`) additionally grants `NOPASSWD` on the daemon binary
itself.

Nothing in the codebase still needs sudo — everything privileged goes through
D-Bus. Both files must go.

### SEC-05 · "Repair & Enable" destroys user data · **VERIFIED** · S0

`bigame-ui/src/app.rs`, in a timer that fires **every 2 seconds** whenever
falcond is not running, offers a button wired to:

```rust
"rm -f /etc/falcond/config.conf; rm -f /usr/share/falcond/profiles/user/*.conf; systemctl enable --now falcond"
```

One click silently deletes every per-game profile the user ever made. The dialog
says "Repair". There is no backup, no confirmation of what will be lost, and the
trigger condition (falcond briefly down) is routine.

### SEC-06 · Signal by name · **VERIFIED** · S2

`pkill -HUP falcond` signals *any* process whose name matches. Use the unit
(`systemctl reload falcond`) or the PID from the service manager.

---

## 2. The global configuration has never worked

### CFG-01 · Wrong falcond config path · **VERIFIED** · S1

```rust
pub const CONFIG_PATH: &str = "/etc/falcond/falcond.conf";
```

falcond 2.0.2 reads **`/etc/falcond/config.conf`**. Confirmed two ways:

```
$ strings /usr/sbin/falcond | grep /etc/falcond
/etc/falcond/config.conf
$ ls /etc/falcond/
config.conf          # and nothing else
```

Consequences, both live on this machine:

- `config::read()` always fails → the Tuning view silently shows `Default`
  values instead of the real configuration (which is
  `scx_sched_props = gaming, profile_mode = handheld`).
- `apply_falcond_config` writes `/etc/falcond/falcond.conf`, a file falcond
  never opens, then `SIGHUP`s falcond, which re-reads `config.conf` and finds it
  unchanged.

So **every global setting in the Tuning page — scheduler, scheduler mode,
V-Cache mode, profile mode, poll interval — is a no-op.** The UI reports
success. This is the single largest placebo in the project and it is exactly
what the brief asked to be found.

The stale `profile_mode = handheld` in `config.conf` on a desktop is the
fingerprint of the older sudo-based path that did work; the D-Bus migration
broke it.

### CFG-02 · V-Cache path disagreement · **VERIFIED** · S1

| Location | Path |
|---|---|
| `bigame-core/src/vcache.rs` | `…/amd_x3d_vcache/**AMDI0101:00**/amd_x3d_mode` |
| `bigame-daemon/src/main.rs` | `…/amd_x3d_vcache/**AMDI0015:00**/amd_x3d_mode` |

They can never agree: the UI decides whether to *offer* V-Cache using one path
and the daemon decides whether to *write* it using another. Both hardcode an ACPI
instance id that varies per board. The driver directory must be globbed.

Not reproducible on this bench (no X3D part) — `NOT TESTED — hardware
unavailable` for the write path, but the inconsistency is a certainty from the
source.

---

## 3. Booster Mode

### BST-01 / BST-02 / BST-03 · **VERIFIED** · S1

`widgets/booster_toggle.rs` is 55 lines, and this is all of it:

```rust
let profile = if row.is_active() { "performance" } else { "balanced" };
gio::spawn_blocking(move || { let _ = bigame_core::dbus::power_profile_set(&target); });
```

- It is an `AdwSwitchRow`, exactly what the brief asked not to have.
- "On" is one D-Bus property write. Nothing else in the entire system is touched.
- "Off" hardcodes `balanced`. On this bench the resting profile is
  `performance`, so one toggle cycle **permanently degrades** the machine's
  baseline. If the user were on `power-saver`, same story.
- The return value is discarded (`let _ =`), so a failed write still flips the
  switch and still turns the row green. The UI asserts success it never checked.
- There is no snapshot, no plan, no verification, no report, no rollback, no
  persistence across a crash.

This is the feature to rebuild, and `KEEP` is not an option for any part of it.

---

## 4. Gamescope

### GS-01 · Invalid flag · **VERIFIED** · S1

`gamescope::Config::to_args()` emits `--fsr`. Gamescope 3.16.28 removed it —
`--help` lists only `-F, --filter` plus `--sharpness, --fsr-sharpness`, and the
shared prefix makes `--fsr` collide with `--fsr-sharpness`:

```
$ gamescope --fsr --backend headless -- true
gamescope: invalid value for --fsr-sharpness, "--backend" is either not an
integer or is far too large
$ gamescope -F fsr --backend headless -- true
[gamescope] Info console: gamescope version 3.16.28    # accepted
```

`launcher.rs` already builds `-F fsr` correctly. So the project contains a
correct builder and a broken one; `Config::to_args()`, `build_command()` and
`launch()` are the broken, unused pair (**GS-03**) and should be deleted rather
than fixed.

### GS-02 · No capability detection · **ANALYSIS** · S2

Nothing anywhere runs `gamescope --version` or probes which flags exist. The
brief explicitly requires this, and GS-01 is precisely the failure mode it
prevents. `--hdr-enabled`, `--adaptive-sync`, `--mangoapp`, `-F`, `--backend`
all vary by build.

### GS-04 · Missing window mode · **VERIFIED** · S2

`build_gamescope_argv` never emits `-f` or `-b`. In nested mode (which is what
runs under a KDE Wayland session) gamescope therefore opens a small decorated
window instead of fullscreen.

Also note `gamescope::Config.framerate_limit` is mapped to `-r`, which is
`--nested-refresh` — the nested display's refresh rate, not a limiter. It does
cap frames in nested mode, so it is not broken, but the field name and the UI
label promise something else, and the real `--framerate-limit` flag (a *divisor*
of the refresh rate) is never used.

---

## 5. Telemetry

### TEL-01 · GPU telemetry aborts early · **VERIFIED** · S1

```rust
for card_entry in drm_dir.flatten() {
    let hwmon_base = card_entry.path().join("device/hwmon");
    let hwmon_dir = std::fs::read_dir(&hwmon_base).ok()?;   // <-- returns from the FUNCTION
```

`/sys/class/drm` contains connector nodes (`card1-DP-1`, `card1-HDMI-A-1`,
`version`, …) that have no `device/hwmon`. The first one encountered makes
`.ok()?` return `None` from `amd_hwmon_read_u64` entirely — the loop never
reaches the remaining cards. Readdir order is not stable, so GPU temperature and
clock are reported intermittently or never. On this bench `/sys/class/drm` holds
3 connector nodes and 2 cards.

The `?` must be a `continue`.

### TEL-02 · Wrong GPU on dual-GPU systems · **VERIFIED** · S2

Even once TEL-01 is fixed, the function returns the **first** card with a
readable value. On this bench that is `card0`, the Cezanne iGPU that renders
nothing, while the RX 9060 XT on `card1` is the card the user cares about — and
the only one exposing `power1_average`. The GPU must be selected by role (the
device backing the connected outputs / the highest-capability render node), not
by readdir order.

---

## 6. Game detection and profiles

### GAME-01 · Detected games can never match · **VERIFIED** · S1

`games.rs::parse_acf` stores the Steam **`installdir`** in the `executable`
field, and `profiles` then keys the falcond profile on it. falcond matches
`/proc/<pid>/comm` — confirmed by `strings /usr/sbin/falcond`
(`/proc/%i/comm`, `/proc/%i/exe`, `matched pid= name='' profile='`) and by the
stock profile names it ships: `Cyberpunk2077.exe`, `cs2`,
`Civ7_linux_Vulkan_FinalRelease`, `ffxiv_dx11.exe` — all process names.

The two user profiles this app previously wrote on this machine are
`Arc Raiders.conf` and `Dead by Daylight.conf` — display titles with spaces.
falcond loads them (`loaded 2 user profile overrides`) and can never match them
to a process. **Profiles created through the UI's game list do nothing.**

### GAME-02 · No Proton awareness · **ANALYSIS** · S2

Detection is manifest-scraping only. There is no process-tree walk, no Steam
AppID correlation, no Proton/Wine prefix inspection, so the intermediate
processes Proton spawns are neither recognised nor skipped.

### PROF-01 · Per-game setting applied globally · **VERIFIED** · S2

`profiles::save()` ends with:

```rust
if !profile.cpu_governor.is_empty() {
    proxy.set_cpu_governor(&profile.cpu_governor)   // every core, right now, forever
}
```

Editing a *per-game* profile immediately changes the *system-wide* governor, at
edit time rather than at game launch, with no record of the previous value and
no path back.

### PROF-02 · falcond never reloads after save · **VERIFIED** · S1

`delete()` sends `SIGHUP`; `save()` does not. A newly saved profile is not
picked up until falcond restarts.

### PROF-03 · `sudo -n` from the GUI · **VERIFIED** · S2

```rust
std::process::Command::new("sudo").args(["-n", "/usr/bin/pkill", "-HUP", "falcond"]).status().ok();
```

Blocks the GTK main thread on a subprocess, depends on the sudoers file that
SEC-04 requires deleting, and swallows the result. `sudo -n true` already fails
on this machine ("uma senha é necessária"), so this call is dead in practice.

---

## 7. Scheduler

### SCX-01 / SCX-02 · **VERIFIED** · S2

`sched::Scheduler` is a closed enum of five: `None Bpfland Lavd Rusty Flash`.
This bench has **16** installed (`beerland cake chaos cosmos flow forge layered
mlfq p2dq pandemonium rustland tickless` are all invisible to the UI).
`detect_installed()` does scan `/usr/bin/scx_*` and returns strings, so the enum
is redundant *and* lossy.

More important: nothing detects that the scheduler cannot actually be changed.
falcond delegates to `org.scx.Loader`, which is not running here —

```
falcond[5569]: warning(scx_loader): Failed to load initial state: ServiceUnknown
$ cat /sys/kernel/sched_ext/state
disabled
$ which scxctl   # not installed
```

— yet the Tuning page offers a scheduler picker with no indication that every
selection will be silently discarded. Capability detection must gate this.

---

## 8. Launch pipeline

### LNCH-01 · Hidden coupling to the power profile · **VERIFIED** · S2

```rust
fn is_turbo_mode_active() -> bool {
    …
    crate::dbus::power_profile_get().map(|p| p.eq_ignore_ascii_case("performance")).unwrap_or(false)
}
```

If this returns false, `LaunchPlan` drops Gamescope, Wine FSR, vkBasalt and all
frame-generation env vars on the floor and logs at `info`. A user who turns
Booster off loses their upscaler with no visible cause. Presentation-layer
settings should not be gated on a CPU power policy.

### LNCH-02 · Steam bypass · **VERIFIED** · S2

`is_steam_applaunch_command` returns early for `steam -applaunch`, skipping the
whole pipeline "for stability". Since Steam is how most games start, the video
pipeline is inert for the common case. `video_config::write_env_file` partially
compensates via `environment.d`, but that only covers env vars, never Gamescope,
and only after re-login.

---

## 9. Smaller findings

- **DBUS-01** (S3) — `dbus::service::run()` re-reads `/tmp/falcond_status` every
  500 ms for the process lifetime and diffs the whole string. That is 2 wakeups
  per second forever, in an app whose stated goal is to stay out of the game's
  way. `inotify` on the file, or falcond's own bus, is the right mechanism.
  The status file itself is a root-owned `0644` file in world-writable `/tmp`;
  it is only read (never written) by this project, so the symlink risk is
  falcond's, but `/run/falcond` — which falcond's own shipped config already
  names as `status_dir` — is the correct location to consume.
- **T-01** (S2) — `test_launch_plan_*` depend on `is_turbo_mode_active()`, which
  falls through to live D-Bus. They pass here only because the bench sits in
  `performance`. Tests must not read system state.
- **FMT-01** (S3) — `cargo fmt --check` fails on `examples/force_save.rs` and
  `src/launcher.rs`.
- **REPO-01** (S3) — the tree carries two `*.pkg.tar` archives, a `pkg/`
  makepkg staging dir, `.pytest_cache/`, `messages.mo` at the root, a bare git
  dir `bigame-mode/`, and a **complete nested copy of the project** under
  `src/bigame-mode/` including 30 `.po~` editor backups. Only 119 files are
  tracked, so most of this is untracked clutter, but `src/bigame-mode/` shadows
  real source paths and confuses every search tool.
- **Dead code** — `gamescope::launch()`, `gamescope::Config::build_command()`,
  `Config::to_args()`, `profiles::export/import`, `models::FrameGenMode`,
  `UpscalingSettings::{base,target}_*` versus `gamescope::Config::{width,height}`
  are two parallel resolution models that never meet (**GS-03**).

---

## 10. Conclusions that drive the rewrite

1. **The project's two headline features do not work.** Global configuration
   writes to the wrong file (CFG-01); profiles created from detected games key on
   a name falcond cannot match (GAME-01). Both fail silently and both report
   success.
2. **The privileged surface is unsafe in three independent ways** (SEC-01,
   SEC-02/03, SEC-04), any one of which is a local root compromise.
3. **Booster Mode is a single D-Bus property write** with a destructive,
   state-losing "off" path.
4. **Nothing in the codebase verifies that anything it did took effect**, which
   is precisely why CFG-01 and GAME-01 survived to production.

That last point is the architectural lesson: the fix is not to patch the paths,
it is to make "did this actually change the system?" a mandatory step of every
operation. That is what [05-BOOSTER-ARCHITECTURE.md](05-BOOSTER-ARCHITECTURE.md)
specifies and what the new engine implements.
