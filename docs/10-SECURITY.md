# 10 — Security

## 1. Threat model

```
UI (user)  →  bigame-core  →  system bus  →  bigame-daemon (root)  →  sysfs, /etc/falcond
                                   │
                                   └──→ Polkit
```

The trust boundary is the system bus. Everything on the UI side is attacker
controlled from the daemon's point of view — not because the UI is malicious,
but because **an attacker does not have to use the UI.** Anything that can open
the system bus can send the same messages.

That single observation is what the previous design got wrong, and it invalidated
every client-side check it had.

## 2. What was found

### 2.1 Unauthenticated root D-Bus service — **critical**

`bigame-daemon` owned `com.biglinux.BiGameMode` on the system bus and exposed
`save_profile`, `delete_profile`, `apply_falcond_config`, `set_vcache_mode` and
`set_cpu_governor`. **None** checked the caller.

The bus policy granted everybody access:

```xml
<policy context="default">
  <allow send_destination="com.biglinux.BiGameMode"/>
</policy>
```

`polkit.rs` was the whole authorization layer — five string constants, never
referenced anywhere. `com.biglinux.BiGameMode.policy` declared five `auth_admin`
actions that nothing consulted. The actions existed; nothing enforced them.

### 2.2 Path traversal to arbitrary root write — **critical**

```rust
let file_path = target_dir.join(format!("{}.conf", name));
fs::write(&file_path, json_payload)
```

`name` came straight off the bus, unvalidated. Reproduced with the exact
`Path::join` semantics the daemon used:

```
name "../../../../../etc/cron.d/pwn"
  -> writes /etc/cron.d/pwn.conf
name "../../../../../etc/systemd/system/pwn.service"
  -> writes /etc/systemd/system/pwn.service.conf
```

With §2.1, that is **unauthenticated local privilege escalation to root** with
fully attacker-controlled file contents. `delete_profile` had the same flaw for
`remove_file`.

The UI-side `profiles::critical_errors()` did reject `..` — in the client, which
an attacker simply does not use.

### 2.3 Passwordless root through sudoers — **critical**

```
Cmnd_Alias BIGAME_WRITE = /usr/bin/tee /usr/share/falcond/profiles/user/*
%wheel ALL=(root) NOPASSWD: ... BIGAME_WRITE ...
```

`sudoers(5)` is explicit:

> A forward slash (‘/’) will not be matched by wildcards used in the file name
> portion of the command. **When matching the command line arguments, however, a
> slash does get matched by wildcards.**

So `sudo tee /usr/share/falcond/profiles/user/../../../../../etc/sudoers` matched
the rule and ran passwordless as root. `BIGAME_SYSFS` and `BIGAME_DELETE` had the
same shape, and a second file granted `NOPASSWD` on the daemon binary itself.

### 2.4 Root code execution through profile scripts — **critical**

falcond runs as `User=root` and spawns `start_script` / `stop_script` through
`/bin/sh` — verified on the installed binary. A profile write is therefore a way
to run arbitrary code as root, with a delayed trigger: the next time the matching
game starts.

This was not a bug in the old code so much as an unexamined consequence of it.
With §2.1 it composed into the same escalation by a slower path.

### 2.5 Destructive "repair" — **critical (data)**

A timer firing **every 2 seconds** whenever falcond was not running offered a
button wired to:

```
rm -f /etc/falcond/config.conf; rm -f /usr/share/falcond/profiles/user/*.conf; systemctl enable --now falcond
```

One click silently deleted every per-game profile the user had ever made. The
dialog said "Repair". No backup, no statement of what would be lost, and a
trigger condition — falcond briefly down — that is entirely routine.

### 2.6 Signal by name

`pkill -HUP falcond` signals *any* process whose name matches.

## 3. What was done

### 3.1 Authorize every method

Every method now calls `polkit::check` before touching anything.

The caller is identified by its **unique bus name**, not its PID. A PID can be
recycled between a message being sent and being checked; a unique bus name
cannot, because the bus guarantees it is never reused. This is the scheme Polkit
documents for D-Bus services precisely to close that race.

**Failure is denial.** If Polkit is unreachable, the request is refused:

```rust
let authority = AuthorityProxy::new(connection).await.map_err(|e| {
    tracing::error!(action, error = %e, "polkit authority is unreachable");
    zbus::fdo::Error::AccessDenied("authorization service unavailable".into())
})?;
```

A helper that grants root because its authorization service is down is worse
than one that stops working.

### 3.2 Validate server side

All argument checking moved into the root process, and it is allow-listing
rather than deny-listing. Denying known-bad patterns invites an encoding nobody
thought of; permitting only a known-good character set does not.

| Input | Rule |
|---|---|
| Profile name | `[A-Za-z0-9 ._+-]`, no leading `.`, no `..`, ≤128 bytes, no padding |
| Payload | ≤64 KiB, no NUL |
| Governor / EPP | `[a-z0-9_-]`, ≤32 bytes |
| DRM card | `card` + 1–3 digits, exactly |
| DPM level | one of the driver's eight values |
| V-Cache mode | `frequency` or `cache` |

`a_valid_name_can_never_escape_its_directory` is a property test: for every
accepted name, the joined and normalized path must still have the profile
directory as its parent. The §2.2 payloads are asserted rejected by name.

### 3.3 Refuse script hooks outright

`validate::profile_payload` rejects `start_script` and `stop_script`. An
administrator can still place them directly in `/usr/share/falcond/profiles/`,
which correctly requires root to begin with — but "manage my game settings" no
longer implies "run code as root later".

This removes a capability the UI previously exposed. That is the intended
trade: a feature whose only mechanism is delayed root execution, reachable
through a user-level permission, is not one to keep.

### 3.4 Split the policy by consequence

| Action | Active session | Reasoning |
|---|---|---|
| `set-cpu` | `yes` | bounded, reversible, validated against a fixed value set; no worse than what power-profiles-daemon already grants the same user |
| `set-gpu` | `yes` | same |
| `set-vcache` | `yes` | same |
| `write-config` | `auth_admin_keep` | writes a file root falcond acts on system-wide |
| `manage-profiles` | `auth_admin_keep` | profiles are input to a root process |

Remote and inactive sessions always authenticate.

Requiring a password to turn Booster Mode on would make the feature unusable,
and users would go back to running the tweaks by hand as root — a worse outcome
than the one the prompt was protecting against. Configuration and profiles,
which feed a root daemon, are held to a root-level bar.

### 3.5 Delete both sudoers files

Nothing needs sudo any more. Removed, along with the `sudo -n pkill` call in
`profiles::delete` that depended on them — and which had been failing silently on
any normally configured machine anyway.

### 3.6 Atomic writes, and reload through systemd

Configuration is written to a temp file in the same directory and `rename(2)`d
after `fsync`, so falcond never reads a half-written config and a crash cannot
truncate the file that was already there.

falcond is reloaded with `systemctl reload-or-restart falcond.service` rather
than by signalling everything that shares its name.

### 3.7 Confine the helper

```
NoNewPrivileges=yes           ProtectSystem=strict
ProtectHome=yes               ReadWritePaths=/etc/falcond /usr/share/falcond/profiles
ProtectKernelModules=yes                     /sys/devices/system/cpu /sys/class/drm
RestrictAddressFamilies=AF_UNIX              /sys/bus/platform/drivers/amd_x3d_vcache
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources @mount @debug @obsolete
MemoryDenyWriteExecute=yes
```

`PrivateTmp` is deliberately **not** set: falcond publishes status to
`/tmp/falcond_status` and a private `/tmp` would hide it. That is not
hypothetical — the `falcond.service` this project used to install set
`PrivateTmp=yes`, which would have hidden the status file from the very UI that
reads it. That unit is no longer installed at all; falcond owns its own.

### 3.8 Allow-list the bus policy

`deny send_destination` by default, then allow the interface plus introspection
and properties explicitly. Polkit is the real control; this is the outer layer,
written so a future interface added to the object is not automatically exposed
to every local uid the moment it is written.

### 3.9 Journal permissions

`0600` in a `0700` directory. It records what the machine looked like, which is
not sensitive, but it is also what rollback trusts — and a file another user can
write is a file that can send the restore somewhere wrong.

## 4. Not fixed in this pass

* **`/tmp/falcond_status`.** Root-owned `0644` in world-writable `/tmp`. This
  project only ever reads it, so the symlink risk belongs to falcond, but
  `/run/falcond` — which falcond's own shipped config already names as
  `status_dir` — is the correct place to consume. Changing it requires
  coordinating with falcond and was out of scope here.
* **Profile import.** `profiles::import` parses a user-supplied TOML and sends
  it to the helper. The helper validates it like any other payload, so the
  escalation path is closed, but the file dialog accepts any file and the error
  path could be friendlier.
* **Log hygiene.** `tracing` output is reviewed for secrets and carries none —
  knob names, values and paths only. There is no automated check that it stays
  that way.

## 5. Residual risk

With the changes above, a local unprivileged attacker can:

* call the helper and receive an authentication prompt they cannot satisfy;
* if they *are* an active-session user, change CPU/GPU performance knobs — the
  same capability power-profiles-daemon already grants, bounded to validated
  values on fixed paths, and reversible;
* not write outside the two permitted directories;
* not cause code to run as root through a profile.

The highest remaining risk is `write-config` and `manage-profiles` for a user
who can pass the admin prompt — which is, correctly, a user who is already an
administrator.
