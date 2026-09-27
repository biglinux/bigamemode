//! The computer's DNS server, changed through `NetworkManager`, with a backup.
//!
//! Only on request, and only the way the user could do it themselves: the
//! DNS of the connection profile that carries the default route is changed
//! with `nmcli` (as the user — `NetworkManager`'s own Polkit rules decide), then
//! reapplied to its device. No root helper is involved, and nothing is written
//! outside the user's state directory.
//!
//! * **Backup first.** Before the first change to a connection its DNS
//!   settings are saved to `$XDG_STATE_HOME/bigame-mode/network/<uuid>.json`.
//!   A second change keeps that backup — the settings from before
//!   BiGame-mode — so restoring always goes back to where the user started.
//! * **Only what changes.** Just the properties of the chosen server's address
//!   family are written (`ipv4.*` for an IPv4 server, `ipv6.*` for IPv6):
//!   `dns` with the server first and any addresses set by hand after it, and
//!   `ignore-auto-dns yes` so the ones DHCP hands out do not come before it.
//! * **Verified, not assumed.** The profile is read back, the device's DNS
//!   is read back, and so is what the system resolver uses (`resolvectl` under
//!   systemd-resolved, `/etc/resolv.conf` otherwise). A VPN such as Tailscale
//!   can own `/etc/resolv.conf`; the result then says so rather than claiming
//!   the lookups go to the chosen server.
//!
//! `nmcli` is run with an argument vector and `LC_ALL=C`, never through a
//! shell, and its terse output is parsed — never its translated text.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::UserError;
use crate::text::{N_, Text};

/// A `NetworkManager` connection profile that is active on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// The profile's name, as the network settings show it.
    pub name: String,
    /// Its UUID: what every command addresses, and the backup's file name.
    pub uuid: String,
    /// `802-3-ethernet`, `802-11-wireless`, …
    pub kind: String,
    /// The device it is active on, e.g. `enp7s0`.
    pub device: String,
}

/// The DNS settings of one connection profile that this module changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsSettings {
    /// `ipv4.dns`, verbatim, in order.
    pub ipv4_dns: Vec<String>,
    /// `ipv4.ignore-auto-dns`.
    pub ipv4_ignore_auto_dns: bool,
    /// `ipv6.dns`, verbatim, in order.
    pub ipv6_dns: Vec<String>,
    /// `ipv6.ignore-auto-dns`.
    pub ipv6_ignore_auto_dns: bool,
}

/// What was there before BiGame-mode changed a connection's DNS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    /// The connection's UUID.
    pub uuid: String,
    /// Its name when the backup was taken, to show.
    pub name: String,
    /// Unix time the backup was taken.
    pub taken_at: u64,
    /// The server applied last.
    pub applied: String,
    /// The settings to restore.
    pub previous: DnsSettings,
}

/// Whether the computer's DNS can be changed here, and on which connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// It can: this connection carries the default route.
    Ready(Connection),
    /// It cannot, and why.
    Unavailable(Text),
}

/// Whether the system resolver uses what `NetworkManager` was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemResolver {
    /// The resolver lists the server.
    Uses,
    /// It lists other servers: something else (a VPN, a hand-written
    /// `/etc/resolv.conf`) decides where lookups go.
    Other(Vec<String>),
    /// It could not be read.
    Unknown,
}

/// What an apply or a restore left in place, read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The connection changed.
    pub connection: Connection,
    /// The DNS servers its device uses now.
    pub device_dns: Vec<String>,
    /// Whether the device's DNS already reflects the change.
    pub device_ok: bool,
    /// What the system resolver uses; for a restore, [`SystemResolver::Unknown`].
    pub system: SystemResolver,
}

// ── Running nmcli ────────────────────────────────────────────────────────────

/// How `nmcli` is run: the real one, or a fake in the tests.
trait Nmcli {
    /// Run `nmcli` with `args`; its standard output, or an error that carries
    /// its message.
    fn run(&self, args: &[&str]) -> Result<String>;
}

struct SystemNmcli;

impl Nmcli for SystemNmcli {
    fn run(&self, args: &[&str]) -> Result<String> {
        let out = std::process::Command::new("nmcli")
            .args(args)
            // Terse output is not translated, but its error messages and
            // yes/no values could be in a future version; C keeps them fixed.
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .output()
            .with_context(|| format!("run nmcli {}", args.join(" ")))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let message = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        anyhow::bail!(
            "nmcli {}: {}",
            args.first().copied().unwrap_or_default(),
            if message.is_empty() {
                format!("exit status {}", out.status)
            } else {
                message
            }
        )
    }
}

/// Whether an executable `program` is on `PATH`.
fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join(program))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Split one line of `nmcli -t` output (escaping on) into its fields: `:`
/// separates them, and a `:` or `\` inside a value comes as `\:` or `\\`.
#[must_use]
pub fn terse_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            ':' => fields.push(String::new()),
            other => {
                // An escaped character stands for itself.
                let c = if other == '\\' {
                    chars.next().unwrap_or(other)
                } else {
                    other
                };
                if let Some(f) = fields.last_mut() {
                    f.push(c);
                }
            }
        }
    }
    fields
}

/// The active connections from `nmcli -t -f NAME,UUID,TYPE,DEVICE connection
/// show --active`.
#[must_use]
pub fn parse_active(text: &str) -> Vec<Connection> {
    text.lines()
        .filter_map(|line| {
            let mut f = terse_fields(line).into_iter();
            Some(Connection {
                name: f.next()?,
                uuid: f.next()?,
                kind: f.next()?,
                device: f.next()?,
            })
        })
        .filter(|c| !c.uuid.is_empty())
        .collect()
}

/// `key:value` lines from `nmcli -t --escape no -f … connection show` (or
/// `device show`), in order. The key never holds a `:`; an IPv6 value does,
/// which is why escaping is off and only the first `:` splits.
fn properties(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}

/// A property list: `a,b` (`NetworkManager` 1.x) as separate entries.
fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "--")
        .map(str::to_owned)
        .collect()
}

fn yes(value: &str) -> bool {
    matches!(value, "yes" | "true" | "1")
}

/// The DNS settings in `nmcli -t --escape no -f ipv4.dns,ipv4.ignore-auto-dns,
/// ipv6.dns,ipv6.ignore-auto-dns connection show <uuid>` output.
///
/// # Errors
/// Returns an error if a property is missing: settings half read must never
/// become a backup.
pub fn parse_settings(text: &str) -> Result<DnsSettings> {
    let props = properties(text);
    let get = |key: &str| {
        props
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .ok_or_else(|| anyhow::anyhow!("nmcli did not report {key}"))
    };
    Ok(DnsSettings {
        ipv4_dns: list(get("ipv4.dns")?),
        ipv4_ignore_auto_dns: yes(get("ipv4.ignore-auto-dns")?),
        ipv6_dns: list(get("ipv6.dns")?),
        ipv6_ignore_auto_dns: yes(get("ipv6.ignore-auto-dns")?),
    })
}

/// The DNS servers a device uses, from `nmcli -t --escape no -f
/// IP4.DNS,IP6.DNS device show <device>` (`IP4.DNS[1]:1.1.1.1`, …).
#[must_use]
pub fn parse_device_dns(text: &str) -> Vec<String> {
    properties(text)
        .into_iter()
        .filter(|(k, _)| k.starts_with("IP4.DNS") || k.starts_with("IP6.DNS"))
        .flat_map(|(_, v)| list(v))
        .collect()
}

/// The servers in `resolvectl dns <device>` output
/// (`Link 2 (enp7s0): 1.1.1.1 8.8.8.8`).
#[must_use]
pub fn parse_resolvectl(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_once("): ").map(|(_, v)| v))
        .flat_map(str::split_whitespace)
        .map(str::to_owned)
        .collect()
}

/// The address in a DNS entry: `NetworkManager` accepts `1.1.1.1#name` for DNS
/// over TLS, and the part before `#` is the server.
fn entry_address(entry: &str) -> Option<IpAddr> {
    entry.split('#').next()?.trim().parse().ok()
}

// ── The change itself ────────────────────────────────────────────────────────

/// The settings that make `server` the connection's first DNS server:
/// `original` — the settings before BiGame-mode — with `server` first in its
/// family's list, the addresses set by hand after it, and that family's
/// automatic DNS ignored. The other family is left as it is in `current`.
#[must_use]
pub fn with_primary(original: &DnsSettings, current: &DnsSettings, server: IpAddr) -> DnsSettings {
    let first = |list: &[String]| {
        let mut out = vec![server.to_string()];
        out.extend(
            list.iter()
                .filter(|e| entry_address(e) != Some(server))
                .cloned(),
        );
        out
    };
    let mut next = current.clone();
    if server.is_ipv4() {
        next.ipv4_dns = first(&original.ipv4_dns);
        next.ipv4_ignore_auto_dns = true;
    } else {
        next.ipv6_dns = first(&original.ipv6_dns);
        next.ipv6_ignore_auto_dns = true;
    }
    next
}

/// The `nmcli connection modify` arguments that turn `from` into `to`: only
/// the properties that differ. Empty when there is nothing to change.
#[must_use]
pub fn modify_args(uuid: &str, from: &DnsSettings, to: &DnsSettings) -> Vec<String> {
    let yes_no = |b: bool| if b { "yes" } else { "no" }.to_owned();
    let mut props: Vec<String> = Vec::new();
    if from.ipv4_dns != to.ipv4_dns {
        props.extend(["ipv4.dns".to_owned(), to.ipv4_dns.join(",")]);
    }
    if from.ipv4_ignore_auto_dns != to.ipv4_ignore_auto_dns {
        props.extend([
            "ipv4.ignore-auto-dns".to_owned(),
            yes_no(to.ipv4_ignore_auto_dns),
        ]);
    }
    if from.ipv6_dns != to.ipv6_dns {
        props.extend(["ipv6.dns".to_owned(), to.ipv6_dns.join(",")]);
    }
    if from.ipv6_ignore_auto_dns != to.ipv6_ignore_auto_dns {
        props.extend([
            "ipv6.ignore-auto-dns".to_owned(),
            yes_no(to.ipv6_ignore_auto_dns),
        ]);
    }
    if props.is_empty() {
        return props;
    }
    let mut args: Vec<String> = ["connection", "modify", "uuid", uuid]
        .into_iter()
        .map(str::to_owned)
        .collect();
    args.extend(props);
    args
}

/// Whether `uuid` looks like a UUID: hex digits and dashes only. It names a
/// file, so nothing else may reach the path.
#[must_use]
pub fn valid_uuid(uuid: &str) -> bool {
    uuid.len() == 36
        && uuid.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Where backups are kept.
#[must_use]
pub fn backup_dir() -> PathBuf {
    crate::paths::state_home()
        .join("bigame-mode")
        .join("network")
}

fn backup_file(dir: &Path, uuid: &str) -> Result<PathBuf> {
    anyhow::ensure!(valid_uuid(uuid), "not a connection UUID: {uuid:?}");
    Ok(dir.join(format!("{uuid}.json")))
}

fn load_backup_in(dir: &Path, uuid: &str) -> Result<Option<Backup>> {
    let path = backup_file(dir, uuid)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(
            serde_json::from_str(&text).with_context(|| format!("read {}", path.display()))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e).context(format!("read {}", path.display()))),
    }
}

/// The backup kept for a connection, if any.
///
/// # Errors
/// Returns an error if the backup exists and cannot be read: a damaged
/// backup is not "no backup", or the next apply would take BiGame-mode's own
/// values as the ones to restore.
pub fn load_backup(uuid: &str) -> Result<Option<Backup>> {
    load_backup_in(&backup_dir(), uuid)
}

/// Write a backup whole or not at all: a temporary file, then a rename.
fn save_backup_in(dir: &Path, backup: &Backup) -> Result<()> {
    let path = backup_file(dir, &backup.uuid)?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.tmp", backup.uuid));
    std::fs::write(&tmp, serde_json::to_vec_pretty(backup)?)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .with_context(|| format!("write {}", path.display()))
}

fn read_settings(nm: &dyn Nmcli, uuid: &str) -> Result<DnsSettings> {
    parse_settings(&nm.run(&[
        "-t",
        "--escape",
        "no",
        "-f",
        "ipv4.dns,ipv4.ignore-auto-dns,ipv6.dns,ipv6.ignore-auto-dns",
        "connection",
        "show",
        "uuid",
        uuid,
    ])?)
}

fn device_dns(nm: &dyn Nmcli, device: &str) -> Vec<String> {
    nm.run(&[
        "-t",
        "--escape",
        "no",
        "-f",
        "IP4.DNS,IP6.DNS",
        "device",
        "show",
        device,
    ])
    .map(|t| parse_device_dns(&t))
    .unwrap_or_default()
}

/// Make `NetworkManager` use a profile's saved settings on its device now.
/// `device reapply` changes DNS without taking the link down; only if it
/// refuses is the connection brought up again, which does.
fn reapply(nm: &dyn Nmcli, connection: &Connection) -> Result<()> {
    if connection.device.is_empty() {
        return Ok(()); // not active: it takes effect when it next starts
    }
    if let Err(first) = nm.run(&["device", "reapply", &connection.device]) {
        tracing::info!(error = %first, "device reapply refused; bringing the connection up again");
        nm.run(&["connection", "up", "uuid", &connection.uuid])
            .map_err(|e| e.context(first.to_string()))?;
    }
    Ok(())
}

/// Wait briefly for the device to report `want` among its DNS servers (or, with
/// `None`, just read them).
fn settle_device(
    nm: &dyn Nmcli,
    device: &str,
    expected: Option<&str>,
    patience: Duration,
) -> (Vec<String>, bool) {
    let deadline = std::time::Instant::now() + patience;
    loop {
        let now = device_dns(nm, device);
        let ok = expected.is_none_or(|w| now.iter().any(|d| d == w));
        if ok || std::time::Instant::now() >= deadline {
            return (now, ok);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn apply_in(
    nm: &dyn Nmcli,
    dir: &Path,
    connection: &Connection,
    server: IpAddr,
    wait: Duration,
) -> Result<Outcome> {
    anyhow::ensure!(valid_uuid(&connection.uuid), "not a connection UUID");
    let current = read_settings(nm, &connection.uuid)?;
    // The backup is of the settings before BiGame-mode, taken once: a
    // backup already there is kept, with only the server applied updated.
    let backup = Backup {
        applied: server.to_string(),
        ..load_backup_in(dir, &connection.uuid)?.unwrap_or_else(|| Backup {
            uuid: connection.uuid.clone(),
            name: connection.name.clone(),
            taken_at: crate::unix_now(),
            applied: String::new(),
            previous: current.clone(),
        })
    };
    save_backup_in(dir, &backup).map_err(|e| {
        UserError::plain(N_(
            "Could not save a backup of the current settings, so nothing was changed",
        ))
        .caused_by(e)
    })?;
    let wanted = with_primary(&backup.previous, &current, server);
    let args = modify_args(&connection.uuid, &current, &wanted);
    if !args.is_empty() {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        nm.run(&args).map_err(|e| {
            UserError::plain(N_("NetworkManager did not accept the change")).caused_by(e)
        })?;
    }
    reapply(nm, connection).map_err(|e| {
        UserError::plain(N_(
            "The setting was saved, but NetworkManager could not apply it now",
        ))
        .caused_by(e)
    })?;
    let saved = read_settings(nm, &connection.uuid)?;
    if saved != wanted {
        anyhow::bail!(UserError::plain(N_(
            "NetworkManager reads back different settings than the ones written"
        )));
    }
    let (device_dns, device_ok) =
        settle_device(nm, &connection.device, Some(&server.to_string()), wait);
    Ok(Outcome {
        connection: connection.clone(),
        device_dns,
        device_ok,
        system: SystemResolver::Unknown,
    })
}

fn restore_in(
    nm: &dyn Nmcli,
    dir: &Path,
    connection: &Connection,
    wait: Duration,
) -> Result<Option<Outcome>> {
    let Some(backup) = load_backup_in(dir, &connection.uuid)? else {
        return Ok(None);
    };
    let current = read_settings(nm, &connection.uuid)?;
    let args = modify_args(&connection.uuid, &current, &backup.previous);
    if !args.is_empty() {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        nm.run(&args).map_err(|e| {
            UserError::plain(N_("NetworkManager did not accept the previous settings")).caused_by(e)
        })?;
    }
    reapply(nm, connection).map_err(|e| {
        UserError::plain(N_(
            "The previous settings were saved, but NetworkManager could not apply them now",
        ))
        .caused_by(e)
    })?;
    let saved = read_settings(nm, &connection.uuid)?;
    if saved != backup.previous {
        // The backup stays, so restoring can be tried again.
        anyhow::bail!(UserError::plain(N_(
            "NetworkManager reads back different settings than the ones restored"
        )));
    }
    let path = backup_file(dir, &connection.uuid)?;
    std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    let (device_dns, _) = settle_device(nm, &connection.device, None, wait);
    Ok(Some(Outcome {
        connection: connection.clone(),
        device_dns,
        device_ok: true,
        system: SystemResolver::Unknown,
    }))
}

fn availability_with(nm: &dyn Nmcli, installed: bool, route_device: Option<&str>) -> Availability {
    let unavailable = |t: &'static str| Availability::Unavailable(Text::plain(t));
    if !installed {
        return unavailable(N_(
            "NetworkManager is not installed (nmcli was not found), so the DNS cannot be changed from here",
        ));
    }
    let running = nm
        .run(&["-t", "-f", "RUNNING", "general"])
        .is_ok_and(|t| t.trim() == "running");
    if !running {
        return unavailable(N_(
            "NetworkManager is not running, so the DNS cannot be changed from here",
        ));
    }
    let permissions = nm
        .run(&["-t", "general", "permissions"])
        .unwrap_or_default();
    let allowed = |action: &str| {
        terse_fields_pairs(&permissions)
            .find(|(k, _)| k == action)
            .is_none_or(|(_, v)| v != "no")
    };
    if !allowed("org.freedesktop.NetworkManager.settings.modify.system")
        && !allowed("org.freedesktop.NetworkManager.settings.modify.own")
    {
        return unavailable(N_(
            "NetworkManager does not allow this user to change network settings",
        ));
    }
    let Some(device) = route_device else {
        return unavailable(N_(
            "No connection carries the default route: this computer is offline",
        ));
    };
    let active = nm
        .run(&[
            "-t",
            "-f",
            "NAME,UUID,TYPE,DEVICE",
            "connection",
            "show",
            "--active",
        ])
        .map(|t| parse_active(&t))
        .unwrap_or_default();
    match active.into_iter().find(|c| c.device == device) {
        Some(c) if valid_uuid(&c.uuid) => Availability::Ready(c),
        _ => Availability::Unavailable(Text::with(
            N_("The connection to the internet (%s) is not managed by NetworkManager"),
            [device],
        )),
    }
}

/// `key:value` pairs of terse output with escaping on.
fn terse_fields_pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter_map(|l| {
        let mut f = terse_fields(l).into_iter();
        Some((f.next()?, f.next()?))
    })
}

// ── The public calls ─────────────────────────────────────────────────────────

/// Whether the DNS can be changed here, and on which connection: the one
/// `NetworkManager` has active on the device of the default route. Runs
/// `nmcli` a few times; call it off the main thread.
#[must_use]
pub fn availability() -> Availability {
    let route = crate::network::primary_link_brief().map(|l| l.name);
    availability_with(&SystemNmcli, on_path("nmcli"), route.as_deref())
}

/// The connection's DNS settings now.
///
/// # Errors
/// Returns an error if `nmcli` fails or does not report them.
pub fn current(connection: &Connection) -> Result<DnsSettings> {
    read_settings(&SystemNmcli, &connection.uuid)
}

/// What the system resolver uses for lookups on `device`: `resolvectl` when
/// `/etc/resolv.conf` points at systemd-resolved's stub, `/etc/resolv.conf`
/// otherwise.
#[must_use]
pub fn system_resolver(device: &str, server: IpAddr) -> SystemResolver {
    let listed = crate::network::system_resolvers();
    let stub: IpAddr = std::net::Ipv4Addr::new(127, 0, 0, 53).into();
    let servers: Vec<String> = if listed.contains(&stub) {
        match resolvectl_dns(device) {
            Some(s) => s,
            None => return SystemResolver::Unknown,
        }
    } else if listed.is_empty() {
        return SystemResolver::Unknown;
    } else {
        listed.iter().map(ToString::to_string).collect()
    };
    if servers.iter().any(|s| entry_address(s) == Some(server)) {
        SystemResolver::Uses
    } else {
        SystemResolver::Other(servers)
    }
}

/// The servers systemd-resolved uses on `device`.
fn resolvectl_dns(device: &str) -> Option<Vec<String>> {
    let out = std::process::Command::new("resolvectl")
        .args(["dns", device])
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_resolvectl(&String::from_utf8_lossy(&out.stdout)))
}

/// Make `server` the first DNS server of `connection`, keeping a backup of
/// its settings from before BiGame-mode, and read everything back.
///
/// # Errors
/// Returns an error if the backup cannot be written (nothing is changed
/// then), `NetworkManager` refuses the change or cannot apply it, or the
/// settings read back differ from the ones written.
pub fn apply(connection: &Connection, server: IpAddr) -> Result<Outcome> {
    let mut outcome = apply_in(
        &SystemNmcli,
        &backup_dir(),
        connection,
        server,
        Duration::from_secs(3),
    )?;
    outcome.system = system_resolver(&connection.device, server);
    tracing::info!(
        connection = %connection.name,
        %server,
        device_dns = ?outcome.device_dns,
        system = ?outcome.system,
        "DNS server applied"
    );
    Ok(outcome)
}

/// Put back the settings the backup holds and remove it once they read back.
/// `None` when there is no backup for this connection.
///
/// # Errors
/// Returns an error if `NetworkManager` refuses or cannot apply them, or they
/// read back different; the backup is kept then.
pub fn restore(connection: &Connection) -> Result<Option<Outcome>> {
    let outcome = restore_in(
        &SystemNmcli,
        &backup_dir(),
        connection,
        Duration::from_secs(3),
    )?;
    if let Some(o) = &outcome {
        tracing::info!(connection = %connection.name, device_dns = ?o.device_dns, "DNS settings restored");
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fmt::Write as _;

    const UUID: &str = "ef8aa060-05c9-32e9-98f5-671099f06c9a";

    fn wired() -> Connection {
        Connection {
            name: "Wired connection 1".into(),
            uuid: UUID.into(),
            kind: "802-3-ethernet".into(),
            device: "enp7s0".into(),
        }
    }

    /// `NetworkManager` as far as these calls see it: one profile, whose saved
    /// DNS the device takes on when reapplied.
    struct FakeNm {
        saved: RefCell<DnsSettings>,
        device: RefCell<Vec<String>>,
        auto: Vec<String>,
        calls: RefCell<Vec<Vec<String>>>,
        refuse_modify: bool,
    }

    impl FakeNm {
        fn new(saved: DnsSettings) -> Self {
            let fake = Self {
                saved: RefCell::new(saved),
                device: RefCell::new(Vec::new()),
                auto: vec!["192.168.0.1".into()],
                calls: RefCell::new(Vec::new()),
                refuse_modify: false,
            };
            fake.reapply();
            fake
        }

        fn reapply(&self) {
            let s = self.saved.borrow();
            let mut dns = s.ipv4_dns.clone();
            if !s.ipv4_ignore_auto_dns {
                dns.extend(self.auto.iter().cloned());
            }
            dns.extend(s.ipv6_dns.iter().cloned());
            *self.device.borrow_mut() = dns;
        }

        fn modifies(&self) -> Vec<Vec<String>> {
            self.calls
                .borrow()
                .iter()
                .filter(|c| c.get(1).map(String::as_str) == Some("modify"))
                .cloned()
                .collect()
        }
    }

    impl Nmcli for FakeNm {
        fn run(&self, args: &[&str]) -> Result<String> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|a| (*a).to_owned()).collect());
            match args {
                [.., "connection", "show", "uuid", u] if *u == UUID => {
                    let s = self.saved.borrow();
                    let yn = |b: bool| if b { "yes" } else { "no" };
                    Ok(format!(
                        "ipv4.dns:{}\nipv4.ignore-auto-dns:{}\nipv6.dns:{}\nipv6.ignore-auto-dns:{}\n",
                        s.ipv4_dns.join(","),
                        yn(s.ipv4_ignore_auto_dns),
                        s.ipv6_dns.join(","),
                        yn(s.ipv6_ignore_auto_dns),
                    ))
                }
                ["connection", "modify", "uuid", u, props @ ..] if *u == UUID => {
                    anyhow::ensure!(!self.refuse_modify, "Error: not authorized");
                    let mut s = self.saved.borrow_mut();
                    for pair in props.chunks(2) {
                        match pair {
                            ["ipv4.dns", v] => s.ipv4_dns = list(v),
                            ["ipv4.ignore-auto-dns", v] => s.ipv4_ignore_auto_dns = yes(v),
                            ["ipv6.dns", v] => s.ipv6_dns = list(v),
                            ["ipv6.ignore-auto-dns", v] => s.ipv6_ignore_auto_dns = yes(v),
                            other => anyhow::bail!("unexpected {other:?}"),
                        }
                    }
                    Ok(String::new())
                }
                ["device", "reapply", "enp7s0"] => {
                    self.reapply();
                    Ok(String::new())
                }
                [.., "device", "show", "enp7s0"] => {
                    let mut out = String::new();
                    for (i, d) in self.device.borrow().iter().enumerate() {
                        let family = if d.contains(':') { "IP6" } else { "IP4" };
                        let _ = writeln!(out, "{family}.DNS[{}]:{d}", i + 1);
                    }
                    Ok(out)
                }
                other => anyhow::bail!("unexpected nmcli {other:?}"),
            }
        }
    }

    fn manual(dns: &[&str]) -> DnsSettings {
        DnsSettings {
            ipv4_dns: dns.iter().map(|s| (*s).to_owned()).collect(),
            ..DnsSettings::default()
        }
    }

    #[test]
    fn terse_lines_are_split_on_unescaped_colons() {
        assert_eq!(
            terse_fields(r"Wired\: office:ef8a:802-3-ethernet:enp7s0"),
            ["Wired: office", "ef8a", "802-3-ethernet", "enp7s0"]
        );
        assert_eq!(terse_fields(r"a\\b:c"), [r"a\b", "c"]);
        assert_eq!(terse_fields("lone"), ["lone"]);
    }

    #[test]
    fn active_connections_are_parsed_from_terse_output() {
        let text = "Conexão cabeada 1:ef8aa060-05c9-32e9-98f5-671099f06c9a:802-3-ethernet:enp7s0\n\
                    tailscale0:f7087748-28d1-4928-98e7-9ffb5e8a1c07:tun:tailscale0\n\
                    garbage\n";
        let active = parse_active(text);
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].name, "Conexão cabeada 1");
        assert_eq!(active[0].uuid, UUID);
        assert_eq!(active[0].device, "enp7s0");
    }

    #[test]
    fn settings_are_parsed_and_a_missing_one_is_an_error() {
        // As nmcli 1.58 prints them, IPv6 unescaped with --escape no.
        let text = "ipv4.dns:1.1.1.1,1.0.0.1\nipv4.ignore-auto-dns:no\n\
                    ipv6.dns:2606:4700:4700::1111\nipv6.ignore-auto-dns:yes\n";
        let s = parse_settings(text).unwrap();
        assert_eq!(s.ipv4_dns, ["1.1.1.1", "1.0.0.1"]);
        assert!(!s.ipv4_ignore_auto_dns);
        assert_eq!(s.ipv6_dns, ["2606:4700:4700::1111"]);
        assert!(s.ipv6_ignore_auto_dns);
        let empty = parse_settings(
            "ipv4.dns:\nipv4.ignore-auto-dns:no\nipv6.dns:\nipv6.ignore-auto-dns:no\n",
        )
        .unwrap();
        assert_eq!(empty, DnsSettings::default());
        assert!(parse_settings("ipv4.dns:1.1.1.1\n").is_err());
    }

    #[test]
    fn device_and_resolved_servers_are_parsed() {
        let dev = "IP4.DNS[1]:1.1.1.1\nIP4.DNS[2]:192.168.0.1\nIP6.DNS[1]:fe80::1\nIP4.GATEWAY:x\n";
        assert_eq!(parse_device_dns(dev), ["1.1.1.1", "192.168.0.1", "fe80::1"]);
        assert_eq!(
            parse_resolvectl("Link 2 (enp7s0): 1.1.1.1 8.8.8.8\n"),
            ["1.1.1.1", "8.8.8.8"]
        );
        assert!(parse_resolvectl("Link 2 (enp7s0):\n").is_empty());
    }

    #[test]
    fn the_server_goes_first_in_its_family_only() {
        let original = manual(&["1.1.1.1", "1.0.0.1"]);
        let google: IpAddr = "8.8.8.8".parse().unwrap();
        let next = with_primary(&original, &original, google);
        assert_eq!(next.ipv4_dns, ["8.8.8.8", "1.1.1.1", "1.0.0.1"]);
        assert!(next.ipv4_ignore_auto_dns);
        assert!(!next.ipv6_ignore_auto_dns);

        // Already listed: moved to the front, not repeated.
        let cf: IpAddr = "1.0.0.1".parse().unwrap();
        assert_eq!(
            with_primary(&original, &original, cf).ipv4_dns,
            ["1.0.0.1", "1.1.1.1"]
        );

        // An IPv6 server changes only IPv6.
        let v6: IpAddr = "2606:4700:4700::1111".parse().unwrap();
        let next = with_primary(&original, &original, v6);
        assert_eq!(next.ipv4_dns, original.ipv4_dns);
        assert!(!next.ipv4_ignore_auto_dns);
        assert_eq!(next.ipv6_dns, ["2606:4700:4700::1111"]);
        assert!(next.ipv6_ignore_auto_dns);
    }

    #[test]
    fn only_properties_that_change_are_written_as_separate_arguments() {
        let from = manual(&["1.1.1.1"]);
        let mut to = from.clone();
        assert!(modify_args(UUID, &from, &to).is_empty());
        to.ipv4_dns = vec!["8.8.8.8".into(), "1.1.1.1".into()];
        to.ipv4_ignore_auto_dns = true;
        assert_eq!(
            modify_args(UUID, &from, &to),
            [
                "connection",
                "modify",
                "uuid",
                UUID,
                "ipv4.dns",
                "8.8.8.8,1.1.1.1",
                "ipv4.ignore-auto-dns",
                "yes"
            ]
        );
        // Clearing a list is an empty value, not a missing argument.
        let back = modify_args(UUID, &to, &DnsSettings::default());
        assert_eq!(&back[4..], ["ipv4.dns", "", "ipv4.ignore-auto-dns", "no"]);
    }

    #[test]
    fn only_a_uuid_names_a_backup_file() {
        assert!(valid_uuid(UUID));
        for bad in [
            "",
            "../../etc/passwd",
            "ef8aa060-05c9-32e9-98f5-671099f06c9",
            "ef8aa060/05c9-32e9-98f5-671099f06c9a",
            "ef8aa060-05c9-32e9-98f5-671099f06c9g",
        ] {
            assert!(!valid_uuid(bad), "{bad}");
            assert!(backup_file(Path::new("/x"), bad).is_err());
        }
    }

    #[test]
    fn the_backup_format_round_trips() {
        let backup = Backup {
            uuid: UUID.into(),
            name: "Wired".into(),
            taken_at: 1_790_000_000,
            applied: "8.8.8.8".into(),
            previous: manual(&["1.1.1.1", "1.0.0.1"]),
        };
        let json = serde_json::to_string_pretty(&backup).unwrap();
        assert!(json.contains("\"ipv4_ignore_auto_dns\": false"));
        assert_eq!(serde_json::from_str::<Backup>(&json).unwrap(), backup);
    }

    #[test]
    fn apply_backs_up_once_and_restore_puts_the_original_back() {
        let dir = crate::tests::tempdir("dns_apply");
        let original = manual(&["1.1.1.1", "1.0.0.1"]);
        let nm = FakeNm::new(original.clone());
        let google: IpAddr = "8.8.8.8".parse().unwrap();

        let out = apply_in(&nm, &dir, &wired(), google, Duration::ZERO).unwrap();
        assert!(out.device_ok);
        assert_eq!(out.device_dns, ["8.8.8.8", "1.1.1.1", "1.0.0.1"]);
        assert!(nm.saved.borrow().ipv4_ignore_auto_dns);
        let backup = load_backup_in(&dir, UUID).unwrap().unwrap();
        assert_eq!(backup.previous, original);

        // A second server replaces the first; the backup still holds the
        // settings from before BiGame-mode, not the first apply's.
        let quad9: IpAddr = "9.9.9.9".parse().unwrap();
        apply_in(&nm, &dir, &wired(), quad9, Duration::ZERO).unwrap();
        assert_eq!(
            nm.saved.borrow().ipv4_dns,
            ["9.9.9.9", "1.1.1.1", "1.0.0.1"]
        );
        let backup = load_backup_in(&dir, UUID).unwrap().unwrap();
        assert_eq!(backup.previous, original);
        assert_eq!(backup.applied, "9.9.9.9");

        let restored = restore_in(&nm, &dir, &wired(), Duration::ZERO)
            .unwrap()
            .unwrap();
        assert_eq!(*nm.saved.borrow(), original);
        // DHCP's server is back after the ones set by hand.
        assert_eq!(restored.device_dns, ["1.1.1.1", "1.0.0.1", "192.168.0.1"]);
        assert!(load_backup_in(&dir, UUID).unwrap().is_none());
        // Nothing left to restore.
        assert!(
            restore_in(&nm, &dir, &wired(), Duration::ZERO)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_backup_is_written_before_any_change() {
        let dir = crate::tests::tempdir("dns_order");
        let mut nm = FakeNm::new(DnsSettings::default());
        nm.refuse_modify = true;
        let err = apply_in(
            &nm,
            &dir,
            &wired(),
            "8.8.8.8".parse().unwrap(),
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(err.downcast_ref::<UserError>().is_some());
        // Refused: the saved settings are untouched and the backup is there,
        // so a restore is harmless.
        assert_eq!(*nm.saved.borrow(), DnsSettings::default());
        assert!(load_backup_in(&dir, UUID).unwrap().is_some());
        assert_eq!(nm.modifies().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_backup_is_an_error_not_a_missing_one() {
        let dir = crate::tests::tempdir("dns_damaged");
        std::fs::write(dir.join(format!("{UUID}.json")), "{not json").unwrap();
        assert!(load_backup_in(&dir, UUID).is_err());
        let nm = FakeNm::new(manual(&["1.1.1.1"]));
        assert!(
            apply_in(
                &nm,
                &dir,
                &wired(),
                "8.8.8.8".parse().unwrap(),
                Duration::ZERO
            )
            .is_err()
        );
        assert!(nm.modifies().is_empty(), "nothing changed without a backup");
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct Scripted(Vec<(&'static str, &'static str)>);

    impl Nmcli for Scripted {
        fn run(&self, args: &[&str]) -> Result<String> {
            let joined = args.join(" ");
            self.0
                .iter()
                .find(|(k, _)| joined.ends_with(k))
                .map(|(_, v)| (*v).to_owned())
                .ok_or_else(|| anyhow::anyhow!("no answer for {joined}"))
        }
    }

    #[test]
    fn availability_says_why_not() {
        let nm = Scripted(vec![
            ("RUNNING general", "running\n"),
            (
                "general permissions",
                "org.freedesktop.NetworkManager.settings.modify.system:yes\n",
            ),
            (
                "connection show --active",
                "Wired:ef8aa060-05c9-32e9-98f5-671099f06c9a:802-3-ethernet:enp7s0\n",
            ),
        ]);
        assert!(matches!(
            availability_with(&nm, true, Some("enp7s0")),
            Availability::Ready(c) if c.uuid == UUID
        ));
        let reason = |a: Availability| match a {
            Availability::Unavailable(t) => t.english(),
            Availability::Ready(_) => String::new(),
        };
        assert!(reason(availability_with(&nm, false, Some("enp7s0"))).contains("not installed"));
        assert!(reason(availability_with(&nm, true, None)).contains("offline"));
        assert!(reason(availability_with(&nm, true, Some("wlan0"))).contains("wlan0"));
        let stopped = Scripted(vec![("RUNNING general", "asleep\n")]);
        assert!(reason(availability_with(&stopped, true, Some("enp7s0"))).contains("not running"));
        let denied = Scripted(vec![
            ("RUNNING general", "running\n"),
            (
                "general permissions",
                "org.freedesktop.NetworkManager.settings.modify.system:no\n\
                 org.freedesktop.NetworkManager.settings.modify.own:no\n",
            ),
        ]);
        assert!(
            reason(availability_with(&denied, true, Some("enp7s0"))).contains("does not allow")
        );
    }
}
