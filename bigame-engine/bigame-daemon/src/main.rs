//! Big Game Mode privileged helper.
//!
//! A small root service on the system bus that performs the handful of writes
//! the unprivileged UI cannot. Any local process can reach it, so three rules
//! govern everything here:
//!
//! 1. **Authorize first.** Every method calls [`polkit::check`] before doing
//!    anything, and a failure to reach Polkit is a denial, not a bypass.
//! 2. **Validate on this side.** Arguments are allow-listed in [`validate`],
//!    inside the root process. Client-side checks are a usability feature, not
//!    a security boundary.
//! 3. **Write narrowly.** Each method writes one well-known location derived
//!    from validated input — never a path assembled from a caller-supplied
//!    string.

mod backend;
mod polkit;
mod validate;

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::Result;
use bigame_core::profiles::USER_PROFILES_DIR;
use tracing::{Level, error, info, warn};
use tracing_subscriber::FmtSubscriber;
use zbus::{connection, interface};

use polkit::actions;

/// falcond's global configuration file.
///
/// This is `config.conf`, **not** `falcond.conf`: falcond 2.0.2 opens only
/// `/etc/falcond/config.conf` (the only configuration path in its binary), so
/// a setting written anywhere else is silently ignored.
const FALCOND_CONFIG: &str = "/etc/falcond/config.conf";

struct BiGameDaemon {
    connection: zbus::Connection,
    /// Held by every method that writes. zbus runs calls concurrently, and two
    /// writes of one file, or two backend switches, must not interleave: the
    /// second switch would record the first one's result as the prior state.
    writes: tokio::sync::Mutex<()>,
}

impl BiGameDaemon {
    /// Reject the call unless Polkit authorizes this sender for `action`.
    /// Polkit may prompt for a password only if the call says its sender can
    /// wait for one (`ALLOW_INTERACTIVE_AUTHORIZATION` in the header).
    async fn authorize(
        &self,
        hdr: &zbus::message::Header<'_>,
        action: &str,
    ) -> Result<(), zbus::fdo::Error> {
        let interactive = hdr
            .primary()
            .flags()
            .contains(zbus::message::Flags::AllowInteractiveAuth);
        polkit::check(&self.connection, hdr.sender(), action, interactive).await
    }
}

// D-Bus method names are pinned explicitly rather than derived from the Rust
// function names. The derived spelling is not always the obvious one —
// `set_vcache_mode` becomes `SetVcacheMode`, not `SetVCacheMode` — and an
// interface that renames itself because someone tidied a function signature is
// not an interface anyone can depend on.
#[interface(name = "com.biglinux.BiGameMode")]
impl BiGameDaemon {
    /// Write a per-game profile into falcond's user profile directory.
    #[zbus(name = "SaveProfile")]
    async fn save_profile(
        &self,
        name: &str,
        payload: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::MANAGE_PROFILES).await?;
        let _write = self.writes.lock().await;
        validate::profile_name(name).map_err(invalid)?;
        validate::profile_payload(payload).map_err(invalid)?;
        validate::profile_name_matches(name, payload).map_err(invalid)?;
        validate::not_a_root_process(name, &validate::root_process_names(Path::new("/proc")))
            .map_err(invalid)?;

        let dir = Path::new(USER_PROFILES_DIR);
        std::fs::create_dir_all(dir)
            .map_err(|e| failed(&format!("create {}: {e}", dir.display())))?;

        // `name` is validated to contain no separator, so this cannot leave the
        // directory. The assertion documents the invariant the write relies on.
        let path = dir.join(format!("{name}.conf"));
        debug_assert_eq!(path.parent(), Some(dir));

        write_atomic(&path, payload.as_bytes(), 0o644)
            .map_err(|e| failed(&format!("write profile: {e:#}")))?;

        info!(profile = name, "profile saved");
        backend::reload(&self.connection).await;
        Ok(())
    }

    /// Delete a per-game profile.
    #[zbus(name = "DeleteProfile")]
    async fn delete_profile(
        &self,
        name: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::MANAGE_PROFILES).await?;
        let _write = self.writes.lock().await;
        validate::profile_name(name).map_err(invalid)?;

        let path = Path::new(USER_PROFILES_DIR).join(format!("{name}.conf"));
        match std::fs::remove_file(&path) {
            Ok(()) => {
                info!(profile = name, "profile deleted");
                backend::reload(&self.connection).await;
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!(profile = name, "profile already absent");
                Ok(())
            }
            Err(e) => Err(failed(&format!("delete profile: {e}"))),
        }
    }

    /// Replace falcond's global configuration and ask it to reload.
    #[zbus(name = "ApplyFalcondConfig")]
    async fn apply_falcond_config(
        &self,
        config_payload: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::WRITE_CONFIG).await?;
        let _write = self.writes.lock().await;
        validate::config_payload(config_payload).map_err(invalid)?;

        let path = Path::new(FALCOND_CONFIG);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| failed(&format!("create {}: {e}", parent.display())))?;
        }
        let before = std::fs::read_to_string(path).ok();
        write_atomic(path, config_payload.as_bytes(), 0o644)
            .map_err(|e| failed(&format!("write falcond config: {e:#}")))?;

        info!(path = FALCOND_CONFIG, "falcond configuration written");
        if startup_settings(before.as_deref().unwrap_or_default())
            == startup_settings(config_payload)
        {
            backend::reload(&self.connection).await;
        } else {
            info!("a setting falcond reads only at start-up changed; restarting it");
            backend::restart_if_running(&self.connection).await;
        }
        Ok(())
    }

    /// Set the AMD 3D V-Cache mode.
    #[zbus(name = "SetVCacheMode")]
    async fn set_vcache_mode(
        &self,
        mode: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::SET_VCACHE).await?;
        let _write = self.writes.lock().await;
        validate::vcache_mode(mode).map_err(invalid)?;

        // The ACPI instance id in this path is board-specific, so it is found,
        // never hardcoded.
        let Some(path) = find_vcache_attribute() else {
            return Err(zbus::fdo::Error::NotSupported(
                "this CPU has no AMD 3D V-Cache control".into(),
            ));
        };
        std::fs::write(&path, mode)
            .map_err(|e| failed(&format!("write {}: {e}", path.display())))?;
        info!(mode, "V-Cache mode set");
        Ok(())
    }

    /// Set the CPU frequency governor on every online CPU.
    #[zbus(name = "SetCpuGovernor")]
    async fn set_cpu_governor(
        &self,
        governor: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::SET_CPU).await?;
        let _write = self.writes.lock().await;
        validate::cpufreq_value(governor).map_err(invalid)?;
        // Only a governor the kernel lists. Any other name makes cpufreq try
        // to load a `cpufreq_<name>` module, which a caller must not choose.
        offered_by_kernel("scaling_available_governors", governor).map_err(invalid)?;
        write_all_cpus("scaling_governor", governor, "governor")
    }

    /// Set the Energy Performance Preference on every online CPU.
    #[zbus(name = "SetCpuEpp")]
    async fn set_cpu_epp(
        &self,
        epp: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::SET_CPU).await?;
        let _write = self.writes.lock().await;
        validate::cpufreq_value(epp).map_err(invalid)?;
        offered_by_kernel("energy_performance_available_preferences", epp).map_err(invalid)?;
        write_all_cpus("energy_performance_preference", epp, "EPP")
    }

    /// Set `power_dpm_force_performance_level` for one DRM card.
    #[zbus(name = "SetGpuDpmLevel")]
    async fn set_gpu_dpm_level(
        &self,
        card: &str,
        level: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.authorize(&hdr, actions::SET_GPU).await?;
        let _write = self.writes.lock().await;
        validate::drm_card(card).map_err(invalid)?;
        validate::dpm_level(level).map_err(invalid)?;

        let path = PathBuf::from(format!(
            "/sys/class/drm/{card}/device/power_dpm_force_performance_level"
        ));
        if !path.exists() {
            return Err(zbus::fdo::Error::NotSupported(format!(
                "{card} exposes no DPM level control"
            )));
        }
        std::fs::write(&path, level)
            .map_err(|e| failed(&format!("write {}: {e}", path.display())))?;
        info!(card, level, "GPU DPM level set");
        Ok(())
    }

    /// Turn the game performance backend (falcond) on or off, persistently.
    ///
    /// This is Turbo's master switch, and also how control is taken back
    /// after a hand-back: the first call, and the first after a release,
    /// records falcond's state before changing it. Returns the unit's active
    /// state as systemd reports it afterwards.
    #[zbus(name = "SetGameBackend")]
    async fn set_game_backend(
        &self,
        enabled: bool,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<String, zbus::fdo::Error> {
        self.authorize(&hdr, actions::CONTROL_BACKEND).await?;
        let _write = self.writes.lock().await;
        backend::set_enabled(&self.connection, enabled)
            .await
            .map(|state| state.active_state)
            .map_err(|e| failed(&format!("{e:#}")))
    }

    /// Return falcond to the state it was in before Big Game Mode first changed
    /// it, and stop managing it.
    #[zbus(name = "ReleaseGameBackend")]
    async fn release_game_backend(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<bool, zbus::fdo::Error> {
        self.authorize(&hdr, actions::CONTROL_BACKEND).await?;
        let _write = self.writes.lock().await;
        backend::release(&self.connection)
            .await
            .map(|record| record.is_some())
            .map_err(|e| failed(&format!("{e:#}")))
    }

    /// Liveness probe.
    #[zbus(name = "Ping")]
    #[allow(clippy::unused_self)]
    fn ping(&self) -> String {
        "pong".into()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn invalid(reason: String) -> zbus::fdo::Error {
    warn!(reason, "rejected an invalid argument");
    zbus::fdo::Error::InvalidArgs(reason)
}

fn failed(reason: &str) -> zbus::fdo::Error {
    error!(reason, "operation failed");
    zbus::fdo::Error::Failed(reason.to_owned())
}

/// Write a file atomically: temp file in the same directory, then rename.
///
/// A reader — falcond, in every case here — must never observe a partially
/// written configuration, and a crash mid-write must not truncate the file that
/// was already there. The temp file is new and unique (`O_EXCL`, never through
/// a symlink), so nothing that already exists under its name is written into.
fn write_atomic(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent"))?;
    let tmp = dir.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("bigame"),
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(content)?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    // The rename is durable only once the directory entry is. The file has
    // already been replaced, so a failure here is reported, not returned.
    if let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all()) {
        warn!(dir = %dir.display(), error = %e, "could not sync the directory after a write");
    }
    Ok(())
}

/// Whether `value` is one of the words the kernel lists in the cpufreq
/// attribute `list` (`scaling_available_governors`, …) of the first CPU that
/// has it.
fn offered_by_kernel(list: &str, value: &str) -> Result<(), String> {
    let offered = std::fs::read_dir("/sys/devices/system/cpu")
        .map_err(|e| format!("read /sys/devices/system/cpu: {e}"))?
        .flatten()
        .find_map(|entry| std::fs::read_to_string(entry.path().join("cpufreq").join(list)).ok())
        .ok_or_else(|| format!("this system has no {list}"))?;
    if offered.split_whitespace().any(|w| w == value) {
        Ok(())
    } else {
        Err(format!(
            "{value:?} is not offered by the kernel ({})",
            offered.trim()
        ))
    }
}

/// Apply a cpufreq attribute to every online CPU.
///
/// Partial success is reported as failure: a machine with half its cores on one
/// governor is not in the state that was asked for, and the caller's
/// verification step would catch it anyway.
fn write_all_cpus(attr: &str, value: &str, label: &str) -> Result<(), zbus::fdo::Error> {
    let base = Path::new("/sys/devices/system/cpu");
    let entries =
        std::fs::read_dir(base).map_err(|e| failed(&format!("read {}: {e}", base.display())))?;

    let mut written = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(idx) = name.strip_prefix("cpu") else {
            continue;
        };
        if idx.is_empty() || !idx.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let path = entry.path().join("cpufreq").join(attr);
        if !path.exists() {
            continue; // offline CPU, or an attribute this driver does not have
        }
        match std::fs::write(&path, value) {
            Ok(()) => written += 1,
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }

    if written == 0 {
        return Err(zbus::fdo::Error::NotSupported(format!(
            "no CPU exposes {attr} on this system"
        )));
    }
    if !failures.is_empty() {
        return Err(failed(&format!(
            "{label} applied to {written} CPUs but failed on {}: {}",
            failures.len(),
            failures.join("; ")
        )));
    }
    info!(cpus = written, value, "{label} set");
    Ok(())
}

/// Locate the AMD 3D V-Cache attribute by globbing the driver directory.
fn find_vcache_attribute() -> Option<PathBuf> {
    const DRIVER_DIR: &str = "/sys/bus/platform/drivers/amd_x3d_vcache";
    std::fs::read_dir(DRIVER_DIR)
        .ok()?
        .flatten()
        .map(|e| e.path().join("amd_x3d_mode"))
        .find(|p| p.exists())
}

// Four worker threads, not tokio's default of one per CPU: with that, the
// helper's thread count followed the machine (17 tasks on 16 threads), and
// the unit's `TasksMax=64` would stop it starting on a 64-thread processor.
// Writing calls serialise on one lock anyway, and every wait — Polkit, a
// password prompt, systemd settling — is asynchronous.
#[tokio::main(worker_threads = 4)]
async fn main() -> Result<()> {
    // Under systemd the output is the journal, which timestamps every line
    // itself and shows colour codes as `[2m…[0m` in `journalctl`.
    let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let builder = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_ansi(terminal);
    if terminal {
        tracing::subscriber::set_global_default(builder.finish())
    } else {
        tracing::subscriber::set_global_default(builder.without_time().finish())
    }
    .expect("install tracing subscriber");

    info!("starting bigame-daemon");

    // The connection is built first so the interface can hold a handle to it
    // for Polkit calls, then the well-known name is requested once the object
    // is serving — a client that sees the name is guaranteed a live object.
    let connection = connection::Builder::system()?.build().await?;
    connection
        .object_server()
        .at(
            "/com/biglinux/BiGameMode",
            BiGameDaemon {
                connection: connection.clone(),
                writes: tokio::sync::Mutex::new(()),
            },
        )
        .await?;
    connection.request_name("com.biglinux.BiGameMode").await?;

    info!("serving com.biglinux.BiGameMode on the system bus");
    std::future::pending::<()>().await;
    Ok(())
}

/// The settings falcond reads only when it starts, as written in `text`.
///
/// Its SIGHUP reload re-reads the file and the profiles, but the global
/// scheduler and 3D V-Cache mode are applied only at start-up, and
/// `enable_performance_mode` decides at start-up whether it connects to
/// power-profiles-daemon at all (falcond 2.0.2, `daemon.zig`). A change to
/// any of them needs a restart to take effect; anything else, a reload.
fn startup_settings(text: &str) -> Vec<(String, String)> {
    const KEYS: &[&str] = &[
        "enable_performance_mode",
        "scx_sched",
        "scx_sched_props",
        "vcache_mode",
    ];
    // Read the strict way the payload was validated, so a setting cannot sit
    // where this reader misses it and falcond does not.
    let mut found: Vec<(String, String)> = validate::config_settings(text)
        .into_iter()
        .filter(|(k, _)| KEYS.contains(&k.as_str()))
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::startup_settings;

    #[test]
    fn a_change_falcond_reads_only_at_start_up_is_told_apart() {
        let base = "enable_performance_mode = true\nscx_sched = lavd\nscx_sched_props = gaming\nvcache_mode = none\nprofile_mode = none\npoll_interval_ms = 9000\n";
        // Only a reloadable key changed.
        let poll = base.replace("9000", "5000");
        assert_eq!(startup_settings(base), startup_settings(&poll));
        let set = base.replace("profile_mode = none", "profile_mode = htpc");
        assert_eq!(startup_settings(base), startup_settings(&set));
        // Each start-up key changed.
        for (from, to) in [
            ("scx_sched = lavd", "scx_sched = bpfland"),
            ("scx_sched_props = gaming", "scx_sched_props = latency"),
            ("vcache_mode = none", "vcache_mode = cache"),
            (
                "enable_performance_mode = true",
                "enable_performance_mode = false",
            ),
        ] {
            assert_ne!(
                startup_settings(base),
                startup_settings(&base.replace(from, to)),
                "{to}"
            );
        }
        // Quoting does not count as a change.
        assert_eq!(
            startup_settings(base),
            startup_settings(&base.replace("scx_sched = lavd", "scx_sched = \"lavd\""))
        );
    }
}
