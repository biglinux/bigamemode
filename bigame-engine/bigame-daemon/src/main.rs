//! BiGame-mode privileged helper.
//!
//! A small root service on the system bus that performs the handful of writes
//! the unprivileged UI cannot. Its design follows from the audit of its
//! predecessor, which had no authorization, no argument validation, and a path
//! traversal that turned any local uid into root.
//!
//! Three rules govern everything here:
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
/// This is `config.conf`, **not** `falcond.conf`. The audit (CFG-01) found the
/// project writing the latter, which falcond never opens — so every global
/// setting silently did nothing. Confirmed against falcond 2.0.2, whose binary
/// contains the string `/etc/falcond/config.conf` and no other config path.
const FALCOND_CONFIG: &str = "/etc/falcond/config.conf";

struct BiGameDaemon {
    connection: zbus::Connection,
}

impl BiGameDaemon {
    /// Reject the call unless Polkit authorizes this sender for `action`.
    async fn authorize(
        &self,
        hdr: &zbus::message::Header<'_>,
        action: &str,
    ) -> Result<(), zbus::fdo::Error> {
        polkit::check(&self.connection, hdr.sender(), action).await
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
        validate::profile_name(name).map_err(invalid)?;
        validate::profile_payload(payload).map_err(invalid)?;

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
        validate::payload(config_payload).map_err(invalid)?;

        let path = Path::new(FALCOND_CONFIG);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| failed(&format!("create {}: {e}", parent.display())))?;
        }
        // falcond reads `enable_performance_mode` only at start-up (measured on
        // 2.0.2: a reload keeps the power-profiles connection it has), so a
        // change to it needs a restart. Everything else it re-reads on SIGHUP.
        let startup_flag = |text: &str| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix("enable_performance_mode"))
                .map(|rest| rest.trim_start_matches([' ', '=']).trim().to_owned())
        };
        let before = std::fs::read_to_string(path).ok();
        write_atomic(path, config_payload.as_bytes(), 0o644)
            .map_err(|e| failed(&format!("write falcond config: {e:#}")))?;

        info!(path = FALCOND_CONFIG, "falcond configuration written");
        if before.as_deref().and_then(startup_flag) == startup_flag(config_payload) {
            backend::reload(&self.connection).await;
        } else {
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
        validate::vcache_mode(mode).map_err(invalid)?;

        // The ACPI instance id in this path is board-specific — the audit
        // (CFG-02) found the old code hardcoding two *different* ids in two
        // places, so neither the UI nor the helper could agree on the device.
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
        validate::cpufreq_value(governor).map_err(invalid)?;
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
        validate::cpufreq_value(epp).map_err(invalid)?;
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

    /// Liveness probe.
    #[allow(clippy::unused_self)]
    /// Turn the game performance backend (falcond) on or off, persistently.
    ///
    /// This is Turbo's master switch. Returns the unit's active state as
    /// systemd reports it afterwards.
    #[zbus(name = "SetGameBackend")]
    async fn set_game_backend(
        &self,
        enabled: bool,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<String, zbus::fdo::Error> {
        self.authorize(&hdr, actions::CONTROL_BACKEND).await?;
        backend::set_enabled(&self.connection, enabled)
            .await
            .map(|state| state.active_state)
            .map_err(|e| failed(&format!("{e:#}")))
    }

    /// Return falcond to the state it was in before BiGame-mode first changed
    /// it, and stop managing it.
    #[zbus(name = "ReleaseGameBackend")]
    async fn release_game_backend(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<bool, zbus::fdo::Error> {
        self.authorize(&hdr, actions::CONTROL_BACKEND).await?;
        backend::release(&self.connection)
            .await
            .map(|record| record.is_some())
            .map_err(|e| failed(&format!("{e:#}")))
    }

    #[zbus(name = "Ping")]
    async fn ping(&self) -> Result<String, zbus::fdo::Error> {
        Ok("pong".into())
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
/// was already there.
fn write_atomic(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent"))?;
    let tmp = dir.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("bigame"),
        std::process::id()
    ));
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Apply a cpufreq attribute to every online CPU.
///
/// Partial success is reported as failure: a machine with half its cores on one
/// governor is not in the state that was asked for, and the caller's
/// verification step would catch it anyway.
fn write_all_cpus(attr: &str, value: &str, label: &str) -> Result<(), zbus::fdo::Error> {
    let base = Path::new("/sys/devices/system/cpu");
    let entries = std::fs::read_dir(base).map_err(|e| failed(&format!("read {base:?}: {e}")))?;

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

#[tokio::main]
async fn main() -> Result<()> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_max_level(Level::INFO)
            .finish(),
    )
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
            },
        )
        .await?;
    connection.request_name("com.biglinux.BiGameMode").await?;

    info!("serving com.biglinux.BiGameMode on the system bus");
    std::future::pending::<()>().await;
    Ok(())
}
