# Security

Big Game Mode has one privileged component, a small root helper on the system
bus. The UI and AI Graphics run as the user. Two other privileged actions
go through the system's own services, never through Big Game Mode's: installing
a missing package (the distribution's installer) and setting a connection's
DNS servers from the DNS comparison (NetworkManager, `nmcli connection modify
uuid …`, authorised by NetworkManager's own Polkit actions).

```text
UI (user) → bigame-core → system bus → bigame-daemon (root) → sysfs, /etc/falcond, systemd
                              └──→ Polkit
```

The trust boundary is the system bus. From the helper's side everything the
UI sends is attacker-controlled, because an attacker need not use the UI:
checks in the UI are for usability, never for security. The helper follows
three rules: **authorize first, validate on the root side, write narrowly** —
each method writes one well-known location derived from validated input, never
a path the caller assembled.

## Authorization

- Every privileged method asks Polkit before doing anything else. The subject
  is the caller's unique bus name (`system-bus-name`), not its PID, so a
  recycled PID cannot inherit an authorization.
- It fails closed: no identifiable sender, Polkit unreachable or a failed
  check means *access denied*. `Ping` is the only unauthenticated method.
- Polkit may show a password prompt only for a call whose header carries
  D-Bus's `ALLOW_INTERACTIVE_AUTHORIZATION` flag. Big Game Mode's client sets
  it on every privileged method, and `busctl` by default; any other caller
  gets Polkit's answer without a dialog appearing on the user's screen.
- The Polkit texts are translated like the rest of the application: the
  package build merges every catalogue into the policy with `msgfmt --xml`.
- Method names are pinned explicitly, so renaming a Rust function cannot
  rename the D-Bus interface.

| Polkit action | Methods | Active session | Inactive / other |
|---|---|---|---|
| `com.biglinux.bigamemode.control-backend` | SetGameBackend, ReleaseGameBackend | yes | admin |
| `com.biglinux.bigamemode.set-cpu` | SetCpuGovernor, SetCpuEpp | yes | admin |
| `com.biglinux.bigamemode.set-gpu` | SetGpuDpmLevel | yes | admin |
| `com.biglinux.bigamemode.set-vcache` | SetVCacheMode | yes | admin |
| `com.biglinux.bigamemode.write-config` | ApplyFalcondConfig | admin (kept) | admin |
| `com.biglinux.bigamemode.manage-profiles` | SaveProfile, DeleteProfile | admin (kept) | admin |

Performance knobs and Turbo are bounded, reversible and validated against
fixed value sets — no more than power-profiles-daemon already grants an active
user. The falcond configuration and profiles feed a root daemon, so they take
an administrator's password.

## Validation (inside the root process)

| Input | Rule |
|---|---|
| Profile name | `[A-Za-z0-9 ._+-]`, 1–128 bytes, no leading `.`, no `..`, no leading or trailing space — a separator cannot be expressed |
| Profile `name` field | must equal the name it is saved under, so a profile is always found, and removed, by the name it matches. It may not name a system process, which falcond would otherwise treat as a game: an exact-name deny-list (service manager and its daemons, bus and Polkit, login and privilege programs, display managers, compositors, sound, network, hardware and power daemons, shells, Big Game Mode and falcond), compared without case and also in its 15-byte `comm` form, as falcond matches; and, when saving, no name falcond would match against a process running as root at that moment (its `comm`, or the name falcond takes from its command line). Under `ProtectProc=invisible` the helper sees root processes whose ids are all 0 and that are dumpable, and kernel threads; the deny-list is the guarantee, the `/proc` check widens it. Deleting is never restricted |
| Profile content | ≤ 64 KiB; no NUL or other control characters (a bare `\r` is a line break to some parsers); **exactly one plain `key = value` per line**: falcond's parser (`otter_conf`) reads the next key on the same line after a value, so `idle_inhibit = true start_script = "…"` would hide a second assignment from a line-based check. Keys are `[a-z_][a-z0-9_]*`; a value is a bare word (`[A-Za-z0-9_.+-]`) or a quoted string with no quote, backslash or `#` inside, and nothing may follow it. Boolean settings take only `true` or `false`, and `poll_interval_ms` only 100–600000. Keys come from an allow-list — falcond's fields and Big Game Mode's own — and none is repeated (one parser keeps the first value, another the last); **no `start_script` / `stop_script`**: falcond runs them through `/bin/sh` (2.0.14: as the user that owns the matched process, which is root for a root process) |
| falcond configuration | ≤ 64 KiB, the same one-assignment-per-line grammar (with one-line lists for `system_processes`) and only falcond's configuration keys; the settings that decide between a reload and a restart are read the same strict way |
| Governor / EPP | `[a-z0-9_-]`, and one of the values the kernel lists in `scaling_available_governors` / `energy_performance_available_preferences` — an arbitrary governor name would make cpufreq load a `cpufreq_<name>` module |
| DRM card | `card` followed by 1–3 digits |
| DPM level | `auto` only — the driver's own choice. Nothing in Big Game Mode plans a fixed level, and the method needs no password in an active session, so it must not be able to pin the GPU |
| V-Cache mode | `frequency` or `cache`; the attribute is found by listing the driver directory, never by a hardcoded ACPI id |

- Writes are atomic: a new temporary file in the same directory, created
  exclusively and never through a symlink (`O_EXCL | O_NOFOLLOW`), synced,
  renamed over the target, then the directory is synced.
- Every writing method takes one lock, so concurrent calls cannot interleave
  two writes of a file or two backend switches.
- cpufreq values go to every online CPU; partial success is reported as
  failure.
- `SetGameBackend` leaves a unit already in the state asked for alone
  (enabled and running with no restart by systemd since it started, or
  disabled and stopped), and has systemd reload its unit files only when
  enabling or disabling changed a link: the method needs no password in an
  active session, and in a loop it made PID 1 reload continuously.
- falcond is reloaded with SIGHUP through systemd's `KillUnit`, and restarted
  only when a setting falcond reads at start-up changed: `enable_performance_mode`,
  the global `scx_sched`/`scx_sched_props` and `vcache_mode` (a reload re-reads
  the file but applies none of them).
- The helper runs no external program; systemd is driven through its D-Bus
  API. The only environment variable it honours is the bus library's
  `DBUS_SYSTEM_BUS_ADDRESS`, which systemd does not set for the unit and which
  `tests/daemon-authorization.sh` uses to run it on a private bus.

## Sandbox

`data/bigame-daemon.service` (`Type=dbus`) removes everything the helper does
not use:

- `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `ProtectKernelModules`, `ProtectKernelLogs`, `ProtectClock`,
  `ProtectHostname`, `ProtectControlGroups`, `ProtectKernelTunables`,
  `ProtectProc=invisible`,
  `ProcSubset=pid`, `PrivateDevices`, `PrivateTmp`, `RestrictNamespaces`,
  `RestrictRealtime`, `RestrictSUIDSGID`, `LockPersonality`,
  `MemoryDenyWriteExecute`, `RestrictAddressFamilies=AF_UNIX`,
  `SystemCallArchitectures=native`, `SystemCallFilter=@system-service` minus
  `@privileged @resources @mount @debug @obsolete`, `UMask=0022`. No
  `PrivateNetwork`: in its own network namespace the unit's sysfs showed no
  cpufreq attributes (systemd 261, checked), and `AF_UNIX` alone already
  leaves it no network socket.
- `MemoryMax=64M` and `TasksMax=64`: the helper is about 5 MiB resident and
  runs four runtime workers (fixed, not one per CPU), so a leak or a runaway
  loop fails inside its own cgroup.
- `CapabilityBoundingSet=CAP_SYS_ADMIN`: every file the helper writes is
  root's own, so uid 0 needs no capability to write it (`CAP_DAC_OVERRIDE`,
  `CAP_SYS_PTRACE`, `CAP_NET_*`, `CAP_SETUID` and the rest are gone);
  `CAP_SYS_ADMIN` stays because systemd checks a uid-0 caller's capabilities,
  not its uid, before it starts or stops falcond on the caller's behalf.
- `ProtectSystem=strict` leaves `/sys` writable, and on systemd 261 neither
  `ProtectKernelTunables` nor `ReadOnlyPaths=/sys` makes the unit's sysfs
  mount read-only (checked inside a unit). So every top-level `/sys`
  directory except `/sys/devices` is listed in `ReadOnlyPaths`: `/sys/kernel`,
  `/sys/module`, `/sys/power`, drivers' `bind`/`unbind` under `/sys/bus` and
  the rest are out of reach. The cpufreq, DRM and V-Cache attributes the
  helper writes live in `/sys/devices`; the `/sys/class` and `/sys/bus` paths
  it names are symlinks into it. Within it, `/sys/devices/system` (CPU
  hotplug, SMT, microcode reload, pstate limits) and `/sys/devices/virtual`
  (thermal trip points, powercap) are read-only too, with
  `/sys/devices/system/cpu/cpufreq` mounted writable over the first: the
  governor and EPP land there (`cpuN/cpufreq` is a symlink to
  `cpufreq/policyN`), the DPM level in the card's PCI device under
  `/sys/devices/pci*`, the V-Cache mode in the `AMDI0101` platform device
  under `/sys/devices/platform`.
- `/run` is read-only: `ProtectSystem=strict` left it writable on systemd 261
  (`/run/systemd/system`, `/run/udev/rules.d`, removable media under
  `/run/media`), and the helper writes nothing there; connecting to the bus
  socket needs no write access to the mount.
- Writable besides those sysfs attributes: `/etc/falcond`
  (`ConfigurationDirectory=falcond`: falcond's package does not ship it, and
  an absent `ReadWritePaths` entry is ignored, which left it read-only),
  `/usr/share/falcond/profiles/user` and `StateDirectory=bigame-mode`. Only
  the user profiles: the system profiles beside them may carry start scripts
  falcond runs as root, so a path bug in the helper cannot plant one. The
  package owns that directory, so it exists when the helper starts even if
  falcond is installed later.
- Turning falcond on that does not reach `active` disables the unit again,
  so a failed Turbo does not come back on at the next boot; off does not
  disable a unit that is still stopping. A unit found `enabled-runtime` is
  handed back runtime-enabled, not persistently.
- The bus policy lets only root own the name and denies by default, then allows
  the helper's interface plus Introspectable, Properties and Peer, so a future
  interface is not exposed automatically.
- The activation file names `SystemdService=bigame-daemon.service`, so the bus
  never starts the helper outside its sandbox. Package upgrades and removal also
  stop a helper an older release let the bus start directly.

`tests/daemon-authorization.sh` starts the real helper on a private bus as an
ordinary user with no Polkit reachable. The bus has the system bus's defaults
with the shipped bus policy included as it is, so the test also checks that
policy: the helper's interface, Introspectable and Peer get through, an
unlisted interface is refused by the bus. Every privileged method must be
refused by the helper's own Polkit denial, one Polkit check logged per call;
invalid arguments (path traversal, a fixed DPM level, a script hook) get the
same answer and the log shows no argument examined and nothing written, so
authorization comes first. Argument validation itself is covered by the unit
tests in `bigame-daemon/src/validate.rs`.

## Other inputs

- falcond's status is read only when it is a root-owned regular file (a
  symlink planted in `/tmp` is not followed, a FIFO does not block the
  read), and at most 64 KiB of it — on every path, the session-bus
  `GetStatus` included.
- No command passes through a shell. External programs are run with
  argument vectors, as the user: `curl` and `bsdtar` (AI Graphics, and
  `curl` for vkBasalt's shaders), `journalctl` (Logs), `ping` (to the target
  set in Settings, a leading `-` refused) and `tc` (Details), `nmcli` and
  `resolvectl` (the DNS comparison; `nmcli` only to apply the server the user
  picked), `lspci` (About), `systemctl is-active`, `systemd-run --user`
  (reopening a launcher in the session), `flatpak run`/`flatpak kill`
  (Flatpak launchers), `steam -shutdown`, `kscreen-doctor`/`xrandr` (the main
  screen's size and mode), `gamescope --help` and the version flags of
  `glxinfo`, `vulkaninfo`, `mangohud` and `gamemoded` (capabilities and the
  support report), the `mangohud` wrapper, and the game itself — directly or
  through `steam`, `heroic` or `lutris`. NVIDIA GPU readings come from the
  driver's NVML library, loaded in the unprivileged UI process; no NVIDIA
  program is run.
- Files the UI writes for a launch (the MangoHud copy with a frame cap) go
  in `$XDG_RUNTIME_DIR/bigame-mode` or the user's cache, in a directory
  created `0700` and checked to be the user's own, through an exclusive
  temporary file that never follows a symlink — never a fixed name in a
  shared `/tmp`.
- One action runs something as root outside the helper: when Gamescope or
  vkBasalt is enabled but not installed, *Install Missing Packages* runs
  `pamac-installer <packages>`, or `pkexec pacman -S --needed --noconfirm
  <packages>`, with package names from a fixed list — never text from the
  UI.
- A game's launch command comes from the launcher's own data: Steam's app id,
  or a native executable or script (a Windows `.exe` is never executed
  directly); the program is never guessed from a title. Tools such as
  Gamescope, Steam and MangoHud are found on `PATH`, as for any program the
  user runs.
- Steam's launch options are edited only while Steam is closed, with a backup
  of the file before the first change and a read-back; they are read and
  written in Steam's own escapes, and only the words Big Game Mode recorded
  adding are ever removed. The app id they are filed under must be all
  digits: one from a crafted `appmanifest` in a shared library could
  otherwise write a block for another game.
- OptiScaler's log in a game folder is read for Logs and the support report
  only when it is a plain file (never through a link), and only its last
  4 MiB for a report.

## AI Graphics

**Download.** Only when the user asks. The OptiScaler release a game's
version choice names — by default the tested one (`v0.9.4`, SHA-256
`575cb4df866116093df75af607e37fd70e10f5163e0f23fd5c804142e80ef0ad`) — is
fetched from its GitHub release with `curl --disable --fail --proto =https
--proto-redir =https --max-filesize …`, a connect timeout and a stall limit.
It is hashed before anything reads it; a mismatch deletes it. A release other
than the tested one is accepted only if GitHub marks it stable and publishes
a SHA-256 digest for its one archive, whose name must be plain — that digest
comes from the same GitHub response as the file, so it proves the transfer,
not the source; only the tested release's hash is pinned in the program. The
installed release is found again by that hash, so Repair never uses another
version's files, and the unpacked cache keeps a hash per file, so a damaged
file is unpacked again from the kept archive rather than used. The archive's
listing is checked before anything is unpacked: plain files and folders with
relative names only, and the sizes it lists within 1 GiB; what was written is
measured again after. OptiScaler's own update check is switched off in the configuration
Big Game Mode writes.

**Release list.** To offer updates, the list of releases is read from the
GitHub API (`curl --disable`, HTTPS only, size-capped, 20 s time limit) at
most once a day, and only while a game's AI Graphics page is open — never at
a game's launch. A failed request keeps the saved list. These two are the only
network accesses AI Graphics makes; measurements recorded on this machine are
never sent anywhere.

**Extraction.** `bsdtar` lists the archive first and refuses absolute paths,
`..`, symlinks, hard links and devices; it extracts into a temporary directory
without owners or permissions; the result is walked again (plain files and
directories only, ≤ 1 GiB). Nothing downloaded is executed by Big Game Mode.

**Transactions.**

- Every target must be a plain relative path inside the game folder with no
  symlink on the way, checked again just before the rename.
- Every target is examined before any copy. Every original is backed up and
  verified by SHA-256, and the backup is synced to disk before anything is
  replaced.
- A journal is written before the first change. Each file is placed through
  an exclusive temporary file that never follows a symlink, and is verified
  after it lands.
- Any failure rolls back. An apply interrupted by a crash or power loss is
  rolled back the next time the application starts.
- Removal follows the manifest and the hashes, never file names:
  - a file changed by someone else is left alone;
  - an edited configuration is kept as a copy;
  - a file deleted by a game update gets its original back;
  - a damaged backup is never restored.
- Manifests that name paths outside the game folder, or another game's key,
  are refused.
- The support report masks the home directory, user and host in every file,
  and reads no environment, credentials or Steam configuration.

**Never done.**

- No injection into games with anti-cheat (Easy Anti-Cheat, BattlEye, EA
  Javelin, XIGNCODE3, Ricochet, Tencent ACE, PunkBuster, VAC), in any mode,
  with no override.
- NVIDIA DLSS and Streamline DLLs are never fetched, placed or replaced.
- No NVIDIA-check circumvention.
- Microsoft's Agility SDK copy and AMD's `amdxcffx64.dll` are never placed.
- ReShade binaries are not fetched; RenoDX is detected, not installed.
- Nothing from the leaked "DLSS 5" path exists in the project.
- No self-update, no unverified download, no cleanup by file name.

## Licensing

Big Game Mode is GPL-3.0-or-later and **ships no third-party binary**. Anything
placed in a game is fetched at the user's request from the component's
official release, checked against a known SHA-256 and cached once per machine;
the release's license files stay with it.

What each third-party project allows, read from its own license, and what
Big Game Mode does with it. This is the reading the code follows, not legal
advice.

| Component | License | What Big Game Mode does |
|---|---|---|
| [OptiScaler](https://github.com/optiscaler/OptiScaler) | GPL-3.0 | fetched from its GitHub release when the user presses Apply, SHA-256 checked, placed as a transaction; never bundled |
| [fakenvapi](https://github.com/FakeMichau/fakenvapi) | MIT | inside OptiScaler's release; placed only for a DLSS input on a non-NVIDIA GPU |
| [AMD FidelityFX](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) DLLs | AMD binary license: unmodified binaries with notice, no reverse engineering | arrive inside OptiScaler's release with its `Licenses/FidelityFX_*`; placed when FSR is the output, removed with it |
| [Intel XeSS](https://github.com/intel/xess) DLLs | Intel Simplified Software License: unmodified binaries with notice, no modification even at run time | arrive inside OptiScaler's release; placed only when XeSS is the output; the game's own `libxess.dll` is never replaced |
| Proton's FSR 4 provider (`amdxcffx64.dll`, `amdxc64.dll`) | shipped by Valve inside Proton | detected in the game's prefix; Big Game Mode sets the launch option that makes Proton use it and verifies it in the running game; never copied |
| [DLSS-NR-on-AMD](https://github.com/danielblnc/DLSS-NR-on-AMD) | proprietary: personal, non-commercial use; no redistribution, no bundling in another tool, no modification | detected beside the game, its requirements checked, its official page linked; never fetched, placed or removed (`managed: false`) |
| NVIDIA DLSS / Streamline (`nvngx_dlss*.dll`) | NVIDIA RTX SDK license: only inside an application, not as a stand-alone item, no modification | detected and versions read; never fetched, copied between games or replaced. A neural-rendering model the user has (`nvngx_dlssnr.dll`) is detected; Big Game Mode never says where to get one |
| Microsoft Agility SDK (in OptiScaler's release) | DirectX license, Windows only | never placed: of no use under VKD3D-Proton |
| [lsfg-vk](https://lsfg-vk.dev) | 1.0.0 is GPL-3.0; 2.x (the version BigLinux packages now) is CC BY-NC-ND 4.0 | a system package; Big Game Mode writes entries in its configuration and reads its log, never ships or modifies it. `Lossless.dll` is the user's own, read in place, never copied |
| [ReShade](https://github.com/crosire/reshade) | BSD-3 source; binaries distributed by its site | detected in DLL slots and reported as a conflict; never fetched |
| ReShade shaders for vkBasalt's Nara style ([crosire/reshade-shaders](https://github.com/crosire/reshade-shaders), [CeeJayDK/SweetFX](https://github.com/CeeJayDK/SweetFX)) | each shader file carries its author's licence | fetched only when the user picks that style, from pinned commits on `raw.githubusercontent.com`, each file checked against a SHA-256 in the program, kept in the user's vkBasalt folder; never shipped |
| [RenoDX](https://github.com/clshortfuse/renodx) | MIT | reported as an HDR option that needs ReShade's add-on build; never fetched |
| dgVoodoo 2 | proprietary freeware | detected as a DLL slot owner; not used: DXVK already covers DirectX 9–11 under Proton |

Community tools that manage "DLSS 5" (DLSS5oneclick, MIT; DLSS-5-MANAGER,
no open-source license) were read for their architecture only; no code was
taken from either, and none of their leaked-DLL, NGX-gate or GPU-spoofing
paths exists here.

## Residual risk

- An active-session user can change CPU and GPU knobs and Turbo without a
  password: bounded, reversible, validated — the same class of access as
  power-profiles-daemon.
- A user who passes the administrator prompt can write falcond profiles and
  its configuration, but cannot make falcond run code (script hooks are
  refused). The helper's code writes only falcond's directories, the listed
  sysfs attributes and `/var/lib/bigame-mode`. Its sandbox confines its own
  file writes, but not what it can ask of other services: it is uid 0 on the
  system bus, which systemd authorises without Polkit, so code execution
  inside the helper could start a transient unit that runs unconfined. Within
  the writable parts of `/sys/devices` (devices under `pci*` and `platform`,
  the cpufreq policies, `boost` included) the sandbox does not enforce the
  attribute list either. A
  dedicated user with a Polkit rule limited to `falcond.service` would narrow
  this; the helper's defence today is its small, validated interface.
