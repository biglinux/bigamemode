//! Argument validation for the privileged helper.
//!
//! Everything here runs **inside** the root process, on the server side of the
//! bus. That placement is the entire point: a check in the GUI is bypassed by
//! talking to the bus directly, and an unchecked profile name such as
//! `../../../../../etc/cron.d/pwn` becomes a root-owned file in `/etc/cron.d`.
//!
//! The approach throughout is allow-listing. Denying known-bad patterns invites
//! an encoding that was not thought of; permitting only a known-good character
//! set does not.

/// Longest accepted profile name. Well under `NAME_MAX` once `.conf` is added.
const MAX_PROFILE_NAME: usize = 128;

/// Largest accepted configuration or profile payload (64 KiB).
///
/// Real profiles are a few hundred bytes. An unauthenticated peer never gets
/// this far — authorization comes first — so this bounds what an authorized
/// caller can make the helper write into a file falcond then parses as root.
const MAX_PAYLOAD: usize = 64 * 1024;

/// Validate a per-game profile name.
///
/// Accepts letters, digits, space, `.`, `_`, `-` and `+` — enough for real
/// titles such as `Arc Raiders` and process names such as `Cyberpunk2077.exe`,
/// while making a path separator unrepresentable.
///
/// # Errors
/// Returns a descriptive error when the name could escape the profile
/// directory or is otherwise unusable as a bare filename.
pub fn profile_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name is empty".into());
    }
    if name.len() > MAX_PROFILE_NAME {
        return Err(format!("profile name exceeds {MAX_PROFILE_NAME} bytes"));
    }
    // A leading dot would create a hidden file; a name of "." or ".." is a
    // directory reference rather than a file.
    if name.starts_with('.') {
        return Err("profile name may not start with '.'".into());
    }
    if name.contains("..") {
        return Err("profile name may not contain '..'".into());
    }
    if let Some(bad) = name.chars().find(|c| !is_allowed_name_char(*c)) {
        return Err(format!(
            "profile name contains a forbidden character: {bad:?}"
        ));
    }
    // Trailing whitespace produces surprising filenames and confusing UI.
    if name.trim() != name {
        return Err("profile name has leading or trailing whitespace".into());
    }
    Ok(())
}

fn is_allowed_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | '+')
}

/// Validate a configuration or profile payload.
///
/// # Errors
/// Returns an error if the payload is too large or contains NUL bytes.
pub fn payload(content: &str) -> Result<(), String> {
    if content.len() > MAX_PAYLOAD {
        return Err(format!("payload exceeds {MAX_PAYLOAD} bytes"));
    }
    if content.contains('\0') {
        return Err("payload contains NUL bytes".into());
    }
    Ok(())
}

/// Keys whose value falcond executes.
const SCRIPT_KEYS: &[&str] = &["start_script", "stop_script"];

/// The keys a profile may carry: the fields falcond 2.0.14 reads, minus its
/// script hooks, and Big Game Mode's own per-game settings, which falcond
/// skips. Anything else is refused, so a key a later falcond gives meaning to
/// cannot arrive through this interface unexamined.
const PROFILE_KEYS: &[&str] = &[
    "name",
    "performance_mode",
    "scx_sched",
    "scx_sched_props",
    "vcache_mode",
    "idle_inhibit",
    "dmem_protect",
    "disable_split_lock",
    "fg_multiplier",
    "fg_flow_scale",
    "fg_perf_mode",
    "fg_quality",
    "fg_dll_path",
    "fg_hdr",
    "fg_present_mode",
    "gamescope_mode",
];

/// The keys of falcond's global configuration (2.0.14 `Config`).
const CONFIG_KEYS: &[&str] = &[
    "enable_performance_mode",
    "scx_sched",
    "scx_sched_props",
    "vcache_mode",
    "system_processes",
    "profile_mode",
    "poll_interval_ms",
];

/// Processes no game profile may name: falcond would apply a game's
/// performance mode, scheduler and the rest to the system itself. Exact
/// names, compared without case as falcond compares them, so no game title
/// is caught by a prefix. Kept to programs that are never a game: the
/// service manager and its daemons, the bus and Polkit, login and privilege
/// programs, display servers and compositors, the sound server, network
/// and hardware daemons, shells, and Big Game Mode with what it drives.
///
/// falcond matches a profile against the name it takes from the command
/// line and against `/proc/<pid>/comm`, which the kernel cuts to 15 bytes,
/// so a name is also refused when it is that cut form of an entry here
/// (`systemd-journal` for `systemd-journald`).
const SYSTEM_PROCESSES: &[&str] = &[
    // Service manager and its own daemons.
    "systemd",
    "init",
    "(sd-pam)",
    "systemd-executor",
    "systemd-journald",
    "systemd-logind",
    "systemd-udevd",
    "udevd",
    "systemd-resolved",
    "systemd-networkd",
    "systemd-timesyncd",
    "systemd-oomd",
    "systemd-homed",
    "systemd-userdbd",
    "systemd-userwork",
    "systemd-machined",
    "systemd-swap",
    // Bus, Polkit, accounts.
    "dbus-daemon",
    "dbus-broker",
    "dbus-broker-launch",
    "polkitd",
    "polkit-agent-helper-1",
    "accounts-daemon",
    "rtkit-daemon",
    // Logging in and gaining privileges.
    "login",
    "agetty",
    "getty",
    "sshd",
    "sshd-session",
    "sshd-auth",
    "sudo",
    "su",
    "pkexec",
    "doas",
    "run0",
    "passwd",
    "unix_chkpwd",
    "sddm",
    "sddm-helper",
    "gdm",
    "gdm-session-worker",
    "lightdm",
    "greetd",
    // Display servers, compositors and the desktop shell.
    "xorg",
    "xwayland",
    "kwin_wayland",
    "kwin_wayland_wrapper",
    "kwin_x11",
    "gnome-shell",
    "plasmashell",
    "ksmserver",
    "kded5",
    "kded6",
    "gamescope",
    "gamescope-wl",
    "gamescopereaper",
    "sway",
    "hyprland",
    "labwc",
    "wayfire",
    "niri",
    "weston",
    "cosmic-comp",
    "xfwm4",
    "openbox",
    "picom",
    // Sound.
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "pulseaudio",
    "jackd",
    "jackdbus",
    // Network, devices, power, scheduling.
    "networkmanager",
    "nm-dispatcher",
    "wpa_supplicant",
    "iwd",
    "dhcpcd",
    "dhclient",
    "connmand",
    "modemmanager",
    "bluetoothd",
    "avahi-daemon",
    "firewalld",
    "chronyd",
    "ntpd",
    "crond",
    "cron",
    "atd",
    "cupsd",
    "udisksd",
    "upowerd",
    "colord",
    "fwupd",
    "boltd",
    "switcheroo-control",
    "power-profiles-daemon",
    "tuned",
    "tuned-ppd",
    "thermald",
    "irqbalance",
    "ananicy-cpp",
    "earlyoom",
    "nvidia-persistenced",
    "nvidia-powerd",
    "packagekitd",
    "pamac-daemon",
    "scx_loader",
    "auditd",
    "rsyslogd",
    "syslog-ng",
    "dockerd",
    "containerd",
    "libvirtd",
    "kthreadd",
    // Shells: every script in the session, root's included, runs in one.
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "mksh",
    "tcsh",
    "csh",
    "busybox",
    // Big Game Mode and what it drives.
    "falcond",
    "gamemoded",
    "bigame-daemon",
    "bigame-ui",
];

/// The length the kernel cuts `/proc/<pid>/comm` to.
const COMM_LEN: usize = 15;

/// Whether `name` is one of `names`, or the `comm` form of one: what
/// falcond would match a process of that name by.
fn names_one_of<'a>(name: &str, names: impl IntoIterator<Item = &'a str>) -> bool {
    let name = name.to_ascii_lowercase();
    names.into_iter().any(|known| {
        let known = known.to_ascii_lowercase();
        known == name || (name.len() == COMM_LEN && known.starts_with(&name))
    })
}

/// The names falcond could match each process running as root by, read
/// from `proc` (normally `/proc`): its `comm`, and the name falcond takes
/// from its command line. Lowercase.
///
/// Best effort, on top of [`SYSTEM_PROCESSES`]: a process started after the
/// call is not seen, and under the unit's `ProtectProc=invisible` the helper
/// (uid 0 with no `CAP_SYS_PTRACE`) sees only root processes whose ids are
/// all 0 and that have not made themselves non-dumpable — which is most
/// root services, and every kernel thread. Unreadable entries are skipped:
/// the deny-list above is the guarantee, this only widens it.
#[must_use]
pub fn root_process_names(proc: &std::path::Path) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir(proc) else {
        return names;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let is_pid = entry
            .file_name()
            .to_str()
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !is_pid {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(dir.join("status")) else {
            continue;
        };
        if !runs_as_root(&status) {
            continue;
        }
        if let Ok(comm) = std::fs::read_to_string(dir.join("comm")) {
            let comm = comm.trim_end_matches('\n');
            if !comm.is_empty() {
                names.insert(comm.to_ascii_lowercase());
            }
        }
        if let Ok(cmdline) = std::fs::read(dir.join("cmdline")) {
            let cmdline = String::from_utf8_lossy(&cmdline);
            let name = name_from_cmdline(&cmdline);
            if !name.is_empty() {
                names.insert(name.to_ascii_lowercase());
            }
        }
    }
    names
}

/// Whether a `/proc/<pid>/status` text has a real or effective uid of 0.
fn runs_as_root(status: &str) -> bool {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .is_some_and(|uids| uids.split_whitespace().take(2).any(|u| u == "0"))
}

/// The process name falcond 2.0.x takes from a NUL-separated command line
/// (`selectProcessNameFromCmdline`): the base name, after `/` or a backslash, of
/// the first argument that ends in `.exe`, or else of the first non-empty
/// one.
fn name_from_cmdline(cmdline: &str) -> &str {
    let mut fallback = "";
    for arg in cmdline.split('\0').filter(|a| !a.is_empty()) {
        let base = arg.rsplit(['/', '\\']).next().unwrap_or(arg);
        if base.is_empty() {
            continue;
        }
        if fallback.is_empty() {
            fallback = base;
        }
        if base.len() >= 4
            && base.is_char_boundary(base.len() - 4)
            && base[base.len() - 4..].eq_ignore_ascii_case(".exe")
        {
            return base;
        }
    }
    fallback
}

/// Refuse a profile name falcond would match against a process running as
/// root now, whatever it is called: one in `root_names`, as
/// [`root_process_names`] read them.
///
/// # Errors
/// Returns an error naming the clash.
pub fn not_a_root_process(
    name: &str,
    root_names: &std::collections::HashSet<String>,
) -> Result<(), String> {
    if names_one_of(name, root_names.iter().map(String::as_str)) {
        Err(format!(
            "{name:?} is the name of a process running as root, not a game: a profile may not name it"
        ))
    } else {
        Ok(())
    }
}

/// One `key = value` assignment of falcond's configuration format, read
/// strictly: the key, then the value with its quotes removed.
///
/// falcond's parser (`otter_conf`) does not need a line break after a value:
/// it reads the next identifier on the same line. A reader that looks only
/// at the first `=` of a line would then see one key where falcond sees
/// two — `idle_inhibit = true start_script = "…"` hides a script hook. So
/// every line must be exactly one assignment, and a value is either a bare
/// word or a quoted string with no quote, backslash or `#` inside, after
/// which nothing may follow. With `lists`, a value may also be a one-line
/// list of such strings (`system_processes`).
fn assignment(line: &str, lists: bool) -> Result<Option<(&str, &str)>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let bad = || format!("not one plain `key = value` assignment: {line:?}");
    let (key, rest) = line.split_once('=').ok_or_else(bad)?;
    let key = key.trim_end();
    let first = key.chars().next().ok_or_else(bad)?;
    if !(first.is_ascii_lowercase() || first == '_')
        || !key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(bad());
    }
    let value = rest.trim_start();
    let plain = |s: &str| !s.contains(['"', '\\', '#']);
    let ok = if let Some(inner) = value.strip_prefix('"') {
        inner.strip_suffix('"').is_some_and(plain)
    } else if let Some(items) = value.strip_prefix('[').filter(|_| lists) {
        items.strip_suffix(']').is_some_and(|items| {
            items.trim().is_empty()
                || items.split(',').all(|item| {
                    item.trim()
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .is_some_and(plain)
                })
        })
    } else {
        !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'))
    };
    if !ok {
        return Err(bad());
    }
    Ok(Some((key, value.trim_matches('"'))))
}

/// Settings falcond reads as a boolean.
const BOOL_KEYS: &[&str] = &[
    "enable_performance_mode",
    "performance_mode",
    "idle_inhibit",
    "dmem_protect",
    "disable_split_lock",
    "fg_perf_mode",
    "fg_hdr",
];

/// How often falcond may rescan the process list, in milliseconds: never a
/// busy loop, never effectively off.
const POLL_INTERVAL_MS: std::ops::RangeInclusive<u32> = 100..=600_000;

/// Whether `value` is one `key` can take, where that is known for certain:
/// booleans and falcond's scan interval. The rest keep the general grammar.
fn value_fits(key: &str, value: &str) -> Result<(), String> {
    if BOOL_KEYS.contains(&key) && !matches!(value, "true" | "false") {
        return Err(format!("{key} must be true or false"));
    }
    if key == "poll_interval_ms"
        && !value
            .parse::<u32>()
            .is_ok_and(|ms| POLL_INTERVAL_MS.contains(&ms))
    {
        return Err(format!(
            "poll_interval_ms must be between {} and {}",
            POLL_INTERVAL_MS.start(),
            POLL_INTERVAL_MS.end()
        ));
    }
    Ok(())
}

/// Every assignment of a payload, each key once and from `allowed`.
fn assignments<'a>(
    content: &'a str,
    allowed: &[&str],
    lists: bool,
) -> Result<Vec<(&'a str, &'a str)>, String> {
    payload(content)?;
    if content
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("content contains control characters".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in content.lines() {
        let Some((key, value)) = assignment(line, lists)? else {
            continue;
        };
        if SCRIPT_KEYS.contains(&key) {
            return Err(format!("{key} is not accepted here: falcond executes it"));
        }
        if !allowed.contains(&key) {
            return Err(format!("{key} is not a setting accepted here"));
        }
        if !seen.insert(key) {
            return Err(format!("{key} is given more than once"));
        }
        value_fits(key, value)?;
        out.push((key, value));
    }
    Ok(out)
}

/// Validate falcond's global configuration payload: the generic [`payload`]
/// checks, one plain assignment per line, and only the keys falcond's
/// configuration has.
///
/// # Errors
/// Returns an error for anything [`assignments`] refuses.
pub fn config_payload(content: &str) -> Result<(), String> {
    assignments(content, CONFIG_KEYS, true).map(|_| ())
}

/// The settings of a configuration payload that passed [`config_payload`],
/// read the same strict way.
#[must_use]
pub fn config_settings(content: &str) -> Vec<(String, String)> {
    assignments(content, CONFIG_KEYS, true)
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// Validate a per-game profile payload.
///
/// Beyond the generic [`payload`] checks this rejects `start_script` and
/// `stop_script`. falcond runs as `User=root` and spawns those values through
/// `/bin/sh` (2.0.14: as the user that owns the matched process, which a
/// profile naming a root process makes root), so accepting them would mean
/// any caller authorized to save a profile could run arbitrary code as root
/// the next time the matching process starts — turning a "manage my game
/// settings" permission into a full root escalation with a delayed trigger.
///
/// Script hooks are therefore not writable through this interface at all. An
/// administrator can still place them directly in
/// `/usr/share/falcond/profiles/`, which correctly requires root to begin with.
///
/// falcond has its own parser, so anything the two could read differently is
/// refused outright ([`assignment`]): more than one assignment on a line,
/// quoted keys, control characters (a bare `\r` is a line break to some
/// parsers and not to others), a key given twice (one parser keeps the first
/// value, another the last) and any key outside [`PROFILE_KEYS`].
///
/// # Errors
/// Returns an error for oversized payloads, NUL or other control characters,
/// lines that are not one plain assignment, repeated or unknown keys, or
/// script hooks.
pub fn profile_payload(content: &str) -> Result<(), String> {
    assignments(content, PROFILE_KEYS, false).map(|_| ())
}

/// Validate a CPU governor or Energy Performance Preference value.
///
/// # Errors
/// Returns an error for anything outside `[a-z0-9_-]`.
pub fn cpufreq_value(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 32 {
        return Err("value has an implausible length".into());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err("value contains characters no cpufreq attribute accepts".into());
    }
    Ok(())
}

/// Validate a DRM card node name such as `card1`.
///
/// # Errors
/// Returns an error for anything that is not `card` followed by digits, which
/// makes it impossible to redirect the write to another sysfs path.
pub fn drm_card(card: &str) -> Result<(), String> {
    let Some(rest) = card.strip_prefix("card") else {
        return Err("not a DRM card name".into());
    };
    if rest.is_empty() || rest.len() > 3 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return Err("not a DRM card name".into());
    }
    Ok(())
}

/// The one GPU DPM level the helper writes: `auto`, the driver's own
/// choice. Nothing in Big Game Mode plans another — a fixed level pins the
/// GPU and loses the firmware's boost (BENCHMARKS.md) — and the method needs
/// no password in an active session, so it must not be able to pin `high`,
/// `low` or `manual` for whoever calls it.
const DPM_LEVELS: &[&str] = &["auto"];

/// Validate a GPU DPM level.
///
/// # Errors
/// Returns an error for any value outside [`DPM_LEVELS`].
pub fn dpm_level(level: &str) -> Result<(), String> {
    if DPM_LEVELS.contains(&level) {
        Ok(())
    } else {
        Err(format!("unknown DPM level; expected one of {DPM_LEVELS:?}"))
    }
}

/// Validate an AMD 3D V-Cache mode.
///
/// # Errors
/// Returns an error for anything other than `frequency` or `cache`.
pub fn vcache_mode(mode: &str) -> Result<(), String> {
    if matches!(mode, "frequency" | "cache") {
        Ok(())
    } else {
        Err("V-Cache mode must be 'frequency' or 'cache'".into())
    }
}

/// Require a profile's `name` field to be the name it is saved under.
///
/// falcond matches processes by the `name` field, not by the file name, so
/// without this a caller could save `Cyberpunk2077.exe.conf` containing
/// `name = "Xorg"` and have falcond apply a game profile to the display
/// server. Tying the two together also means a profile can always be found,
/// and removed, by the name it matches.
///
/// # Errors
/// Returns an error when the field is missing or differs from `name`.
pub fn profile_name_matches(name: &str, payload: &str) -> Result<(), String> {
    // Read as [`profile_payload`] reads it, so there is exactly one `name`.
    let field = assignments(payload, PROFILE_KEYS, false)?
        .into_iter()
        .find_map(|(k, v)| (k == "name").then(|| v.to_owned()));
    if names_one_of(name, SYSTEM_PROCESSES.iter().copied()) {
        return Err(format!(
            "{name:?} is a system process, not a game: a profile may not name it"
        ));
    }
    match field {
        Some(f) if f == name => Ok(()),
        Some(f) => Err(format!(
            "the profile's name field ({f:?}) must match the name it is saved as ({name:?})"
        )),
        None => Err("the profile has no name field".into()),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn booleans_and_the_scan_interval_take_only_their_values() {
        assert!(
            config_payload("poll_interval_ms = 9000\nenable_performance_mode = true\n").is_ok()
        );
        for bad in [
            "poll_interval_ms = 0\n",
            "poll_interval_ms = 99999999999\n",
            "poll_interval_ms = fast\n",
            "enable_performance_mode = yes\n",
        ] {
            assert!(config_payload(bad).is_err(), "{bad}");
        }
        let profile = "name = \"game.exe\"\nperformance_mode = 1\n";
        assert!(profile_payload(profile).is_err());
    }

    #[test]
    fn two_assignments_on_one_line_cannot_hide_a_script_or_a_second_name() {
        use super::{profile_name_matches, profile_payload};
        // falcond reads the next key on the same line after a value.
        let hidden = "name = \"Cyberpunk2077.exe\"\nidle_inhibit = true start_script = \"id > /root/x\" name = \"Xorg\"\n";
        assert!(profile_payload(hidden).is_err());
        assert!(profile_name_matches("Cyberpunk2077.exe", hidden).is_err());
        assert!(profile_payload("name = \"a\" stop_script = \"x\"\n").is_err());
        assert!(profile_payload("idle_inhibit = true # start_script = \"x\"\n").is_err());
        // A value with a quote, a backslash or a comment sign inside.
        assert!(profile_payload("fg_dll_path = \"/a\\\"b\"\n").is_err());
        assert!(profile_payload("fg_dll_path = \"/a#b\"\n").is_err());
        // Unknown keys are refused; what Big Game Mode writes is accepted.
        assert!(profile_payload("start_command = \"x\"\n").is_err());
        let written = "name = \"TMNT.exe\"\nperformance_mode = true\nscx_sched = none\nscx_sched_props = default\nvcache_mode = none\nidle_inhibit = true\nfg_multiplier = 1\nfg_flow_scale = 100\nfg_perf_mode = false\nfg_quality = 0\nfg_dll_path = \"/home/u/Lossless Scaling/Lossless.dll\"\nfg_hdr = false\nfg_present_mode = 1\ngamescope_mode = \"auto\"\ndmem_protect = true\ndisable_split_lock = true\n";
        assert!(
            profile_payload(written).is_ok(),
            "{:?}",
            profile_payload(written)
        );
        assert!(profile_name_matches("TMNT.exe", written).is_ok());
    }

    #[test]
    fn a_profile_may_not_name_the_session_itself() {
        use super::profile_name_matches;
        let check = |name: &str| profile_name_matches(name, &format!("name = \"{name}\"\n"));
        for name in [
            "Xorg",
            "kwin_wayland",
            "systemd-logind",
            "NetworkManager",
            "sshd",
            "zsh",
            "fish",
            "gamescope",
            "Hyprland",
            // `comm` cuts names to 15 bytes, and falcond matches that too.
            "systemd-journal",
            "systemd-journald",
            "kwin_wayland_wr",
            "dbus-broker-lau",
        ] {
            assert!(check(name).is_err(), "{name}");
        }
        // Real games, and names that merely start like a system process.
        for name in [
            "vkcube",
            "Cyberpunk2077.exe",
            "X4",
            "sshd-simulator",
            "Gamescope Demo.exe",
            "systemd-journa",
            "SOTTR.exe",
            "factorio",
            "Proton",
        ] {
            assert!(check(name).is_ok(), "{name}: {:?}", check(name));
        }
    }

    #[test]
    fn falcond_reads_the_process_name_from_the_command_line_as_here() {
        use super::name_from_cmdline;
        // falcond's own test cases (scanner.zig).
        assert_eq!(
            name_from_cmdline(
                "/usr/bin/wine64\0C:\\Program Files\\Cyberpunk 2077\\bin\\x64\\Cyberpunk2077.exe\0--fullscreen\0"
            ),
            "Cyberpunk2077.exe"
        );
        assert_eq!(
            name_from_cmdline("/usr/bin/umu-run\0--gameid\x001234\0"),
            "umu-run"
        );
        assert_eq!(
            name_from_cmdline("/usr/lib/systemd/systemd-logind\0"),
            "systemd-logind"
        );
        assert_eq!(name_from_cmdline("\0\0"), "");
        assert_eq!(name_from_cmdline("GAME.EXE\0"), "GAME.EXE");
    }

    #[test]
    fn a_profile_may_not_name_a_process_running_as_root() {
        use super::{not_a_root_process, root_process_names};
        // A /proc of three processes: a root daemon, a root process started
        // through wine, and the user's game.
        let proc =
            std::env::temp_dir().join(format!("bigame-daemon-test-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&proc);
        let process = |pid: u32, uid: u32, comm: &str, cmdline: &str| {
            let dir = proc.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("status"),
                format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
            std::fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(dir.join("cmdline"), cmdline).unwrap();
        };
        process(1, 0, "init-daemon", "/usr/lib/init-daemon\0--system\0");
        process(
            40,
            0,
            "RootTool.exe",
            "/usr/bin/wine\0C:\\tools\\RootTool.exe\0",
        );
        process(41, 0, "very-long-servi", "/usr/bin/very-long-service\0");
        process(
            500,
            1000,
            "Cyberpunk2077.e",
            "/usr/bin/wine64\0Z:\\games\\Cyberpunk2077.exe\0",
        );
        // Not a process directory.
        std::fs::create_dir_all(proc.join("sys")).unwrap();

        let root = root_process_names(&proc);
        std::fs::remove_dir_all(&proc).unwrap();
        for name in [
            "init-daemon",
            "INIT-DAEMON",
            "RootTool.exe",
            "very-long-servi",
            "very-long-service",
        ] {
            assert!(not_a_root_process(name, &root).is_err(), "{name}");
        }
        assert!(not_a_root_process("Cyberpunk2077.exe", &root).is_ok());
        assert!(not_a_root_process("vkcube", &root).is_ok());
        // An unreadable /proc refuses nothing: the deny-list still holds.
        assert!(root_process_names(std::path::Path::new("/nonexistent/proc")).is_empty());
    }

    #[test]
    fn the_global_configuration_is_read_as_strictly() {
        use super::{config_payload, config_settings};
        // This machine's file, and the list Big Game Mode writes.
        let conf = "enable_performance_mode = true\nscx_sched = lavd\nscx_sched_props = gaming\nvcache_mode = none\nprofile_mode = none\npoll_interval_ms = 9000\nsystem_processes = [\"steam\", \"Xwayland\"]\n";
        assert!(config_payload(conf).is_ok());
        assert!(config_settings(conf).contains(&("scx_sched".to_owned(), "lavd".to_owned())));
        // A start-up setting hidden behind a reloadable one.
        assert!(config_payload("poll_interval_ms = 9000 scx_sched = rusty\n").is_err());
        assert!(config_payload("system_processes = [\"a\" ] scx_sched = lavd\n").is_err());
        assert!(config_payload("start_script = \"x\"\n").is_err());
        assert!(config_payload("scx_sched = lavd\nscx_sched = bpfland\n").is_err());
    }

    #[test]
    fn a_profile_that_parsers_could_read_differently_is_refused() {
        use super::profile_payload;
        assert!(profile_payload("name = \"game\"\nidle_inhibit = true\n").is_ok());
        // A bare CR hides a line from `lines()` but not from every parser.
        assert!(profile_payload("name = \"game\"\rstart_script = \"x\"\n").is_err());
        assert!(profile_payload("\"start_script\" = \"x\"\n").is_err());
        assert!(profile_payload("'stop_script' = \"x\"\n").is_err());
        assert!(profile_payload("name = \"game\"\nname = \"Xorg\"\n").is_err());
        assert!(profile_payload("scx_sched = none\nscx_sched = lavd\n").is_err());
        assert!(profile_payload("name = \"game\"\n\tidle_inhibit = true\n").is_ok());
    }

    #[test]
    fn a_profile_can_only_match_the_process_it_is_named_for() {
        assert!(
            profile_name_matches("SOTTR.exe", "name = \"SOTTR.exe\"\nidle_inhibit = true\n")
                .is_ok()
        );
        assert!(profile_name_matches("Cyberpunk2077.exe", "name = \"Xorg\"\n").is_err());
        assert!(profile_name_matches("cs2", "performance_mode = true\n").is_err());
    }

    use super::*;

    #[test]
    fn accepts_real_profile_names() {
        for name in [
            "Cyberpunk2077.exe",
            "Arc Raiders",
            "Dead by Daylight",
            "cs2",
            "Civ7_linux_Vulkan_FinalRelease",
            "ffxiv_dx11.exe",
            "Half-Life 2",
            "C++Builder",
        ] {
            assert!(profile_name(name).is_ok(), "should accept {name:?}");
        }
    }

    #[test]
    fn rejects_path_traversal_payloads() {
        // Traversal names that would write into /etc as root if accepted.
        for name in [
            "../../../../../etc/cron.d/pwn",
            "../../../../../etc/systemd/system/pwn.service",
            "../../../etc/sudoers.d/pwn",
        ] {
            assert!(profile_name(name).is_err(), "must reject {name:?}");
        }
    }

    #[test]
    fn rejects_every_shape_of_path_escape() {
        for name in [
            "..",
            ".",
            "a/b",
            "/absolute",
            "a\\b",
            "..hidden",
            ".hidden",
            "a\0b",
            "a\nb",
            "a\tb",
            // Percent- and URL-style encodings must not slip through either;
            // they are rejected because '%' is simply not on the allow list.
            "%2e%2e%2fetc",
            "a%00b",
        ] {
            assert!(profile_name(name).is_err(), "must reject {name:?}");
        }
    }

    #[test]
    fn rejects_empty_oversized_and_padded_names() {
        assert!(profile_name("").is_err());
        assert!(profile_name(&"a".repeat(MAX_PROFILE_NAME + 1)).is_err());
        assert!(profile_name(&"a".repeat(MAX_PROFILE_NAME)).is_ok());
        assert!(profile_name(" leading").is_err());
        assert!(profile_name("trailing ").is_err());
    }

    #[test]
    fn a_valid_name_can_never_escape_its_directory() {
        // Property check: for every accepted name, joining it under the
        // profile directory must stay inside that directory.
        let base = std::path::Path::new("/usr/share/falcond/profiles/user");
        for name in [
            "Arc Raiders",
            "cs2",
            "Cyberpunk2077.exe",
            "a.b.c",
            "x+y-z_1",
        ] {
            profile_name(name).unwrap();
            let joined = base.join(format!("{name}.conf"));
            let mut normalized = std::path::PathBuf::new();
            for c in joined.components() {
                match c {
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    other => normalized.push(other.as_os_str()),
                }
            }
            assert!(
                normalized.starts_with(base),
                "{name:?} escaped to {}",
                normalized.display()
            );
            assert_eq!(normalized.parent(), Some(base));
        }
    }

    #[test]
    fn profile_payload_rejects_root_script_hooks() {
        // falcond runs as root and spawns these through /bin/sh, so accepting
        // them would make "save a game profile" a root escalation primitive.
        for body in [
            "name = \"x\"\nstart_script = \"/tmp/evil.sh\"\n",
            "name = \"x\"\nstop_script = \"/tmp/evil.sh\"\n",
            "  start_script = \"/tmp/evil.sh\"\n",
            "start_script=\"/tmp/evil.sh\"\n",
        ] {
            assert!(
                profile_payload(body).is_err(),
                "must reject script hook in {body:?}"
            );
        }
    }

    #[test]
    fn profile_payload_accepts_ordinary_profiles() {
        let body = "name = \"Cyberpunk2077.exe\"\n\
                    performance_mode = true\n\
                    scx_sched = none\n\
                    vcache_mode = cache\n\
                    idle_inhibit = true\n";
        assert!(profile_payload(body).is_ok());
    }

    #[test]
    fn profile_payload_ignores_the_words_in_comments_and_values() {
        // A comment mentioning the key, and a value that merely contains the
        // word, are both harmless — only a real assignment is refused.
        assert!(profile_payload("# start_script is not supported\nname = \"x\"\n").is_ok());
        assert!(profile_payload("name = \"my start_script game\"\n").is_ok());
    }

    #[test]
    fn payload_limits() {
        assert!(payload("name = \"x\"\n").is_ok());
        assert!(payload(&"a".repeat(MAX_PAYLOAD)).is_ok());
        assert!(payload(&"a".repeat(MAX_PAYLOAD + 1)).is_err());
        assert!(payload("has\0nul").is_err());
    }

    #[test]
    fn cpufreq_values() {
        assert!(cpufreq_value("performance").is_ok());
        assert!(cpufreq_value("powersave").is_ok());
        assert!(cpufreq_value("balance_performance").is_ok());
        assert!(cpufreq_value("schedutil").is_ok());

        assert!(cpufreq_value("").is_err());
        assert!(
            cpufreq_value("Performance").is_err(),
            "sysfs values are lowercase"
        );
        assert!(cpufreq_value("performance; rm -rf /").is_err());
        assert!(cpufreq_value("../../../etc/shadow").is_err());
        assert!(cpufreq_value(&"a".repeat(33)).is_err());
    }

    #[test]
    fn drm_card_names() {
        assert!(drm_card("card0").is_ok());
        assert!(drm_card("card1").is_ok());
        assert!(drm_card("card127").is_ok());

        // Connector nodes are not cards and must not be writable through here.
        assert!(drm_card("card1-DP-1").is_err());
        assert!(drm_card("card").is_err());
        assert!(drm_card("renderD128").is_err());
        assert!(drm_card("../../../devices").is_err());
        assert!(drm_card("card0/../../..").is_err());
        assert!(drm_card("card99999").is_err());
    }

    #[test]
    fn dpm_levels() {
        assert!(dpm_level("auto").is_ok());
        // Only the driver's own choice: nothing may pin the GPU.
        assert!(dpm_level("high").is_err());
        assert!(dpm_level("manual").is_err());
        assert!(dpm_level("profile_peak").is_err());
        assert!(dpm_level("turbo").is_err());
        assert!(dpm_level("").is_err());
    }

    #[test]
    fn vcache_modes() {
        assert!(vcache_mode("cache").is_ok());
        assert!(vcache_mode("frequency").is_ok());
        assert!(
            vcache_mode("freq").is_err(),
            "the driver spells it 'frequency'"
        );
        assert!(vcache_mode("none").is_err());
    }
}
