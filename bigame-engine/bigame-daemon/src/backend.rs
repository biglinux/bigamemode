//! Control of the game performance backend (falcond) through systemd.
//!
//! Turbo is the master switch: off means falcond does not run, so nothing
//! intervenes in a game; on means it runs and applies profiles. The service
//! is the switch because it is the only one that works — measured against
//! falcond 2.0.2 on the lab VM:
//!
//! * `systemctl stop` with a profile active restores that profile's snapshot
//!   before exiting (falcond's `deinit` deactivates first). Off is a clean
//!   restore.
//! * `enable_performance_mode = false` in its config is honoured only at
//!   start-up; a reload leaves the power-profiles connection it already has,
//!   so that flag cannot be a live switch.
//!
//! systemd, not this process, holds the state. A crash here leaves falcond in
//! whatever state was last set, and enablement persists across reboots, so a
//! Turbo left off stays off.
//!
//! **Ownership.** falcond may have been set up by someone else before
//! BiGame-mode ever touched it. The first time this service changes it, the
//! state it found is recorded in [`OWNERSHIP_RECORD`], and
//! [`release`] puts exactly that back.
//!
//! Everything goes through systemd's D-Bus API rather than `systemctl`, so no
//! process is spawned and no argument ever reaches a shell.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use zbus::zvariant::OwnedObjectPath;

/// The unit this module controls. Fixed: nothing a caller sends selects it.
pub const UNIT: &str = "falcond.service";

/// Where the pre-ownership state is kept. Inside the service's systemd
/// `StateDirectory`, world-readable so the UI can say who owns falcond.
pub const OWNERSHIP_RECORD: &str = "/var/lib/bigame-mode/game-backend.json";

/// How long to wait for systemd to report the state asked for.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Manager {
    #[zbus(name = "StartUnit")]
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    #[zbus(name = "StopUnit")]
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    #[zbus(name = "EnableUnitFiles")]
    #[allow(clippy::type_complexity)]
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Vec<(String, String, String)>)>;
    #[zbus(name = "DisableUnitFiles")]
    fn disable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
    ) -> zbus::Result<Vec<(String, String, String)>>;
    #[zbus(name = "Reload")]
    fn reload(&self) -> zbus::Result<()>;
    #[zbus(name = "GetUnitFileState")]
    fn get_unit_file_state(&self, file: &str) -> zbus::Result<String>;
    #[zbus(name = "LoadUnit")]
    fn load_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    #[zbus(name = "KillUnit")]
    fn kill_unit(&self, name: &str, whom: &str, signal: i32) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
trait Unit {
    #[zbus(property, name = "ActiveState")]
    fn active_state(&self) -> zbus::Result<String>;
}

/// The state falcond was in before BiGame-mode first changed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ownership {
    /// Unix time the record was taken.
    pub taken_at: u64,
    /// systemd unit file state then: `enabled`, `disabled`, `masked`, …
    pub unit_file_state: String,
    /// Whether it was running then.
    pub was_active: bool,
}

impl Ownership {
    fn load() -> Option<Self> {
        let text = std::fs::read_to_string(OWNERSHIP_RECORD).ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// What systemd reports about the unit now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitState {
    /// `enabled`, `disabled`, `masked`, `static`, … or `not-found`.
    pub unit_file_state: String,
    /// `active`, `inactive`, `failed`, `activating`, …
    pub active_state: String,
}

async fn manager(connection: &zbus::Connection) -> zbus::Result<ManagerProxy<'_>> {
    ManagerProxy::new(connection).await
}

/// Read the unit's current state.
///
/// # Errors
/// Returns an error if systemd cannot be reached or the unit does not exist.
pub async fn state(connection: &zbus::Connection) -> zbus::Result<UnitState> {
    let manager = manager(connection).await?;
    let unit_file_state = manager
        .get_unit_file_state(UNIT)
        .await
        .unwrap_or_else(|_| "not-found".into());
    let path = manager.load_unit(UNIT).await?;
    let unit = UnitProxy::builder(connection).path(path)?.build().await?;
    Ok(UnitState {
        unit_file_state,
        active_state: unit.active_state().await?,
    })
}

/// Record what the unit looks like, once, before the first change.
async fn take_ownership(connection: &zbus::Connection) -> anyhow::Result<()> {
    if Path::new(OWNERSHIP_RECORD).exists() {
        return Ok(());
    }
    let found = state(connection).await?;
    let record = Ownership {
        taken_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        unit_file_state: found.unit_file_state,
        was_active: found.active_state == "active",
    };
    let json = serde_json::to_vec_pretty(&record)?;
    crate::write_atomic(Path::new(OWNERSHIP_RECORD), &json, 0o644)?;
    info!(
        ?record,
        "took ownership of the game backend; prior state recorded"
    );
    Ok(())
}

/// Wait for systemd to report `want` (or a terminal failure).
async fn settle(connection: &zbus::Connection, want: &str) -> anyhow::Result<String> {
    let deadline = tokio::time::Instant::now() + SETTLE_TIMEOUT;
    loop {
        let now = state(connection).await?.active_state;
        if now == want || now == "failed" || tokio::time::Instant::now() >= deadline {
            return Ok(now);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Turn the backend on or off, persistently.
///
/// On: enable the unit, then start it. Off: stop it — which restores any game
/// profile it holds — then disable it. The state systemd reports afterwards is
/// returned so the caller can verify rather than assume.
///
/// # Errors
/// Returns an error if systemd refuses the change or the unit does not reach
/// the requested state.
pub async fn set_enabled(
    connection: &zbus::Connection,
    enabled: bool,
) -> anyhow::Result<UnitState> {
    let manager = manager(connection).await?;
    if manager.get_unit_file_state(UNIT).await.is_err() {
        anyhow::bail!("{UNIT} is not installed");
    }
    take_ownership(connection).await?;

    let reached = if enabled {
        manager.enable_unit_files(&[UNIT], false, false).await?;
        manager.reload().await?;
        manager.start_unit(UNIT, "replace").await?;
        settle(connection, "active").await?
    } else {
        manager.stop_unit(UNIT, "replace").await?;
        let reached = settle(connection, "inactive").await?;
        manager.disable_unit_files(&[UNIT], false).await?;
        manager.reload().await?;
        reached
    };
    let now = state(connection).await?;
    let wanted = if enabled { "active" } else { "inactive" };
    if reached != wanted {
        anyhow::bail!("{UNIT} is {reached}, not {wanted}");
    }
    info!(enabled, ?now, "game backend switched");
    Ok(now)
}

/// Put the unit back exactly as it was found, and forget the record.
///
/// # Errors
/// Returns an error if systemd refuses a change. The record is kept in that
/// case so a later attempt can finish.
pub async fn release(connection: &zbus::Connection) -> anyhow::Result<Option<Ownership>> {
    let Some(record) = Ownership::load() else {
        return Ok(None);
    };
    let manager = manager(connection).await?;
    match record.unit_file_state.as_str() {
        "enabled" | "enabled-runtime" => {
            manager.enable_unit_files(&[UNIT], false, false).await?;
        }
        "disabled" => {
            manager.disable_unit_files(&[UNIT], false).await?;
        }
        other => warn!(
            state = other,
            "prior unit file state not restorable; left as is"
        ),
    }
    manager.reload().await?;
    if record.was_active {
        manager.start_unit(UNIT, "replace").await?;
    } else {
        manager.stop_unit(UNIT, "replace").await?;
    }
    std::fs::remove_file(OWNERSHIP_RECORD)?;
    info!(?record, "released the game backend to its prior state");
    Ok(Some(record))
}

/// Ask a running falcond to re-read its configuration and profiles.
///
/// SIGHUP, which falcond handles as a reload. The previous helper used
/// `systemctl reload-or-restart`, and falcond's unit has no `ExecReload`, so
/// every profile save *restarted* it — tearing down the profile of a game that
/// was running. A stopped falcond is left stopped: Turbo decides that, not a
/// profile save.
pub async fn reload(connection: &zbus::Connection) {
    let running = state(connection)
        .await
        .is_ok_and(|s| s.active_state == "active");
    if !running {
        info!("falcond is not running; its files are in place for the next start");
        return;
    }
    match manager(connection).await {
        Ok(m) => match m.kill_unit(UNIT, "main", libc::SIGHUP).await {
            Ok(()) => info!("falcond asked to reload"),
            Err(e) => warn!(error = %e, "could not signal falcond to reload"),
        },
        Err(e) => warn!(error = %e, "systemd unreachable; falcond not reloaded"),
    }
}

/// Restart a running falcond — needed only when a setting it reads at
/// start-up changed (`enable_performance_mode`).
pub async fn restart_if_running(connection: &zbus::Connection) {
    let running = state(connection)
        .await
        .is_ok_and(|s| s.active_state == "active");
    if !running {
        return;
    }
    match manager(connection).await {
        Ok(m) => {
            if let Err(e) = m.stop_unit(UNIT, "replace").await {
                warn!(error = %e, "could not stop falcond for a restart");
                return;
            }
            let _ = settle(connection, "inactive").await;
            if let Err(e) = m.start_unit(UNIT, "replace").await {
                warn!(error = %e, "could not start falcond after a restart");
            }
        }
        Err(e) => warn!(error = %e, "systemd unreachable; falcond not restarted"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ownership_record_round_trips() {
        let record = Ownership {
            taken_at: 1_790_000_000,
            unit_file_state: "enabled".into(),
            was_active: true,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<Ownership>(&json).unwrap(), record);
    }

    #[test]
    fn the_controlled_unit_is_fixed() {
        // Nothing a caller sends chooses the unit; it is a constant.
        assert_eq!(UNIT, "falcond.service");
        assert!(OWNERSHIP_RECORD.starts_with("/var/lib/bigame-mode/"));
    }
}
