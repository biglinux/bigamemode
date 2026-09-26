# Security

BiGame-mode has one privileged component, a small root helper on the system
bus. The UI and AI Graphics run as the user; the one other root action,
installing a missing package, goes through the distribution's own installer.

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
| Profile `name` field | must equal the name it is saved under; otherwise `X.conf` containing `name = "Xorg"` would make falcond treat the display server as a game |
| Profile content | ≤ 64 KiB; no NUL or other control characters (a bare `\r` is a line break to some parsers); keys compared with any quotes removed, and none repeated (one parser keeps the first value, another the last); **no `start_script` / `stop_script`** — falcond runs them as root through `/bin/sh` |
| falcond configuration | ≤ 64 KiB, no NUL |
| Governor / EPP | `[a-z0-9_-]`, and one of the values the kernel lists in `scaling_available_governors` / `energy_performance_available_preferences` — an arbitrary governor name would make cpufreq load a `cpufreq_<name>` module |
| DRM card | `card` followed by 1–3 digits |
| DPM level | one of amdgpu's fixed levels |
| V-Cache mode | `frequency` or `cache`; the attribute is found by listing the driver directory, never by a hardcoded ACPI id |

- Writes are atomic: a new temporary file in the same directory, created
  exclusively and never through a symlink (`O_EXCL | O_NOFOLLOW`), synced,
  renamed over the target, then the directory is synced.
- Every writing method takes one lock, so concurrent calls cannot interleave
  two writes of a file or two backend switches.
- cpufreq values go to every online CPU; partial success is reported as
  failure.
- falcond is reloaded with SIGHUP through systemd's `KillUnit`, and restarted
  only when `enable_performance_mode` changed, which falcond reads at start-up.
- The helper runs no external program and reads no environment variable;
  systemd is driven through its D-Bus API.

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
  `@privileged @resources @mount @debug @obsolete`, `UMask=0022`.
- `ProtectSystem=strict` leaves `/sys` writable, and on systemd 261 neither
  `ProtectKernelTunables` nor `ReadOnlyPaths=/sys` makes the unit's sysfs
  mount read-only (checked inside a unit). So every top-level `/sys`
  directory except `/sys/devices` is listed in `ReadOnlyPaths`: `/sys/kernel`,
  `/sys/module`, `/sys/power`, drivers' `bind`/`unbind` under `/sys/bus` and
  the rest are out of reach. The cpufreq, DRM and V-Cache attributes the
  helper writes live in `/sys/devices`; the `/sys/class` and `/sys/bus` paths
  it names are symlinks into it. Also writable: `/etc/falcond` and
  `/usr/share/falcond/profiles` (ignored when absent), and
  `StateDirectory=bigame-mode`.
- The bus policy lets only root own the name and denies by default, then allows
  the helper's interface plus Introspectable, Properties and Peer, so a future
  interface is not exposed automatically.
- The activation file names `SystemdService=bigame-daemon.service`, so the bus
  never starts the helper outside its sandbox. Package upgrades and removal also
  stop a helper an older release let the bus start directly.

`tests/daemon-authorization.sh` starts the real helper on a private bus as an
ordinary user with no Polkit reachable, and checks that every privileged method
is refused and that nothing is written. Because authorization comes first, its
path-traversal payloads are refused there and never reach argument validation;
that validation is covered by the unit tests in `bigame-daemon/src/validate.rs`.

## Other inputs

- falcond's status is read only when it is a root-owned regular file (a
  symlink planted in `/tmp` is not followed), and at most 64 KiB of it.
- No command passes through a shell. External programs are run with
  argument vectors, as the user: `curl` and `bsdtar` (AI Graphics),
  `journalctl` (Logs), `ping` (to the target set in Settings, a leading `-`
  refused) and `tc` (Details), `lspci` (About), `systemctl is-active`,
  `gamescope --help` and the version flags of `glxinfo`, `vulkaninfo`,
  `mangohud` and `gamemoded` (capabilities and the support report), and the
  game itself. NVIDIA GPU readings come from the driver's NVML library,
  loaded in the unprivileged UI process; no NVIDIA program is run.
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
  and a read-back.

## AI Graphics

**Download.** Only when the user asks. The OptiScaler release a game's
version choice names — by default the tested one (`v0.9.4`, SHA-256
`575cb4df866116093df75af607e37fd70e10f5163e0f23fd5c804142e80ef0ad`) — is
fetched from its GitHub release with `curl --disable --fail --proto =https
--proto-redir =https --max-filesize …`, a connect timeout and a stall limit.
It is hashed before anything reads it; a mismatch deletes it. A release other
than the tested one is accepted only if GitHub marks it stable and publishes
a SHA-256 digest for its one archive, whose name must be plain; the installed
release is found again by that hash, so Repair never uses another version's
files. OptiScaler's own update check is switched off in the configuration
BiGame-mode writes.

**Release list.** To offer updates, the list of releases is read from the
GitHub API (`curl --disable`, HTTPS only, size-capped, 20 s time limit) at
most once a day, and only while a game's AI Graphics page is open — never at
a game's launch. A failed request keeps the saved list. These two are the only
network accesses AI Graphics makes; measurements recorded on this machine are
never sent anywhere.

**Extraction.** `bsdtar` lists the archive first and refuses absolute paths,
`..`, symlinks, hard links and devices; it extracts into a temporary directory
without owners or permissions; the result is walked again (plain files and
directories only, ≤ 1 GiB). Nothing downloaded is executed by BiGame-mode.

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

BiGame-mode is GPL-3.0-or-later and **ships no third-party binary**. Anything
placed in a game is fetched at the user's request from the component's
official release, checked against a known SHA-256 and cached once per machine;
the release's license files stay with it.

What each third-party project allows, read from its own license, and what
BiGame-mode does with it. This is the reading the code follows, not legal
advice.

| Component | License | What BiGame-mode does |
|---|---|---|
| [OptiScaler](https://github.com/optiscaler/OptiScaler) | GPL-3.0 | fetched from its GitHub release when the user presses Apply, SHA-256 checked, placed as a transaction; never bundled |
| [fakenvapi](https://github.com/FakeMichau/fakenvapi) | MIT | inside OptiScaler's release; placed only for a DLSS input on a non-NVIDIA GPU |
| [AMD FidelityFX](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) DLLs | AMD binary license: unmodified binaries with notice, no reverse engineering | arrive inside OptiScaler's release with its `Licenses/FidelityFX_*`; placed when FSR is the output, removed with it |
| [Intel XeSS](https://github.com/intel/xess) DLLs | Intel Simplified Software License: unmodified binaries with notice, no modification even at run time | arrive inside OptiScaler's release; placed only when XeSS is the output; the game's own `libxess.dll` is never replaced |
| Proton's FSR 4 provider (`amdxcffx64.dll`, `amdxc64.dll`) | shipped by Valve inside Proton | detected in the game's prefix; BiGame-mode sets the launch option that makes Proton use it and verifies it in the running game; never copied |
| [DLSS-NR-on-AMD](https://github.com/danielblnc/DLSS-NR-on-AMD) | proprietary: personal, non-commercial use; no redistribution, no bundling in another tool, no modification | detected beside the game, its requirements checked, its official page linked; never fetched, placed or removed (`managed: false`) |
| NVIDIA DLSS / Streamline (`nvngx_dlss*.dll`) | NVIDIA RTX SDK license: only inside an application, not as a stand-alone item, no modification | detected and versions read; never fetched, copied between games or replaced. A neural-rendering model the user has (`nvngx_dlssnr.dll`) is detected; BiGame-mode never says where to get one |
| Microsoft Agility SDK (in OptiScaler's release) | DirectX license, Windows only | never placed: of no use under VKD3D-Proton |
| [lsfg-vk](https://lsfg-vk.dev) | the packaged 1.0.0 is GPL-3.0; the current upstream source is CC BY-NC-ND 4.0 | a system package; BiGame-mode writes entries in its configuration and reads its log, never ships or modifies it. `Lossless.dll` is the user's own, read in place, never copied |
| [ReShade](https://github.com/crosire/reshade) | BSD-3 source; binaries distributed by its site | detected in DLL slots and reported as a conflict; never fetched |
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
  sysfs attributes and `/var/lib/bigame-mode`. Its sandbox is narrower than
  root but does not enforce that list within `/sys/devices`: code execution
  inside the helper could still write other device attributes there.
