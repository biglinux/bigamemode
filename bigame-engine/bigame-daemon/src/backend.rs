//! Control of the game performance backend (falcond) through systemd.
//!
//! Turbo is the master switch: off means falcond does not run, so nothing
//! intervenes in a game; on means it runs and applies profiles. The service
//! is the switch because it is the only one that works (falcond 2.0.2):
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
//! Big Game Mode ever touched it. The first time this service changes it, the
//! state it found is recorded in [`OWNERSHIP_RECORD`], and
//! [`release`] puts exactly that back. A release leaves [`RELEASE_RECORD`]
//! behind, so the UI can tell "handed back" from "never taken"; the next
//! change — Turbo, or taking control back — records the state afresh, exactly
//! as the first one did, and removes it.
//!
//! Everything goes through systemd's D-Bus API rather than `systemctl`, so no
//! process is spawned and no argument ever reaches a shell.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// The unit this module controls. Fixed: nothing a caller sends selects it.
pub const UNIT: &str = "falcond.service";

/// Where the pre-ownership state is kept. Inside the service's systemd
/// `StateDirectory`, world-readable so the UI can say who owns falcond.
pub const OWNERSHIP_RECORD: &str = "/var/lib/bigame-mode/game-backend.json";

/// Written when falcond is handed back, removed when Big Game Mode takes charge
/// again. World-readable, like the ownership record.
pub const RELEASE_RECORD: &str = "/var/lib/bigame-mode/game-backend.released.json";

/// How long to wait for systemd to report the state asked for.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a started falcond has to stay up to count as running.
///
/// systemd calls a `Type=simple` service active as soon as its process
/// exists. falcond 2.0.14 built for x86-64-v3 was "active" on a Sandy Bridge
/// for the few milliseconds before its first BMI2 instruction killed it;
/// systemd restarted it every 100 ms and gave up after five tries, half a
/// second in. Two seconds sees that loop through.
const STARTUP_GRACE: Duration = Duration::from_secs(2);

use bigame_core::systemd::ManagerProxy;

/// The state falcond was in before Big Game Mode first changed it.
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
    /// The record, or `None` when there is none. A record that cannot be read
    /// is an error, not "none": treated as absent, the backend could never be
    /// released, and it would never be re-recorded either.
    fn load() -> anyhow::Result<Option<Self>> {
        match std::fs::read_to_string(OWNERSHIP_RECORD) {
            Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// What a release put back, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Unix time of the release.
    pub released_at: u64,
    /// The state that was restored.
    pub restored: Ownership,
}

pub use bigame_core::systemd::UnitState;

async fn manager(connection: &zbus::Connection) -> zbus::Result<ManagerProxy<'_>> {
    ManagerProxy::new(connection).await
}

/// Read the unit's current state.
///
/// # Errors
/// Returns an error if systemd cannot be reached.
pub async fn state(connection: &zbus::Connection) -> zbus::Result<UnitState> {
    bigame_core::systemd::unit_state(connection, UNIT).await
}

/// Record what the unit looks like, once, before the first change.
async fn take_ownership(connection: &zbus::Connection) -> anyhow::Result<()> {
    if Path::new(OWNERSHIP_RECORD).exists() {
        return Ok(());
    }
    let found = state(connection).await?;
    let record = Ownership {
        taken_at: bigame_core::unix_now(),
        unit_file_state: found.unit_file_state,
        was_active: found.active_state == "active",
    };
    let json = serde_json::to_vec_pretty(&record)?;
    crate::write_atomic(Path::new(OWNERSHIP_RECORD), &json, 0o644)?;
    // Managed again: an earlier hand-back is history, not the current state.
    match std::fs::remove_file(RELEASE_RECORD) {
        Ok(()) => info!("taking charge of a game backend that had been handed back"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!(error = %e, "could not remove the release record"),
    }
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

/// Whether a just-started unit stays up for [`STARTUP_GRACE`]: active the
/// whole time, and not restarted by systemd meanwhile. `None` when it did;
/// otherwise the state it was found in.
async fn stays_active(connection: &zbus::Connection) -> anyhow::Result<Option<String>> {
    let deadline = tokio::time::Instant::now() + STARTUP_GRACE;
    // NRestarts starts again from 0 at a start on request; read it anyway,
    // so the check holds whatever systemd counted before.
    let restarts = |s: &UnitState| s.service.as_ref().map_or(0, |r| r.restarts);
    let first = restarts(&state(connection).await?);
    loop {
        let now = state(connection).await?;
        if now.active_state != "active" || restarts(&now) > first {
            return Ok(Some(now.active_state));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Turn the backend on or off, persistently.
///
/// On: enable the unit, then start it, and see that it stays up. Off: stop it
/// — which restores any game profile it holds — then disable it. The state
/// systemd reports afterwards is returned so the caller can verify rather
/// than assume.
///
/// A unit left `failed` by an earlier run is reset first, both ways. On, so a
/// fixed falcond starts at once instead of being refused for the earlier
/// crashes (systemd's start limit). Off, because stopping a failed unit
/// leaves it failed, and disabling has to happen all the same: left enabled,
/// a falcond that crashes would crash again at every boot. Only ever on this
/// request, never in a loop.
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
        // A build that crashed on this processor crashes again, the same way.
        if bigame_core::falcond_compat::crashes_here() {
            anyhow::bail!(
                "this falcond build crashed on this processor (illegal instruction); \
                 it is not started again until the package changes"
            );
        }
        let before = state(connection).await?;
        if before.is_failed() {
            info!(failure = ?before.failure(), "starting falcond again after a failure; clearing it first");
            manager.reset_failed_unit(UNIT).await?;
        }
        manager.enable_unit_files(&[UNIT], false, false).await?;
        manager.reload().await?;
        let started = match manager.start_unit(UNIT, "replace").await {
            Ok(_) => match settle(connection, "active").await {
                Ok(reached) if reached == "active" => match stays_active(connection).await {
                    Ok(None) => {
                        bigame_core::falcond_compat::forget();
                        Ok(reached)
                    }
                    Ok(Some(now)) => {
                        // Let systemd finish its restarts: one that crashes
                        // at once reaches the start limit within a second
                        // and stays `failed` with the cause (the signal) for
                        // the UI to explain. Stopped earlier, systemd forgets
                        // it. One that is still restarting by then is stopped.
                        let reached = settle(connection, "failed").await?;
                        if reached != "failed" {
                            manager.stop_unit(UNIT, "replace").await?;
                            settle(connection, "inactive").await?;
                        }
                        let failure = state(connection).await?.failure();
                        warn!(state = now, ?failure, "falcond started, then stopped");
                        if failure == Some(bigame_core::systemd::Failure::IllegalInstruction) {
                            match bigame_core::falcond_compat::record() {
                                Ok(()) => warn!(
                                    "falcond's build cannot run on this processor; not starting it again until it changes"
                                ),
                                Err(e) => {
                                    warn!(error = %format!("{e:#}"), "falcond's crash could not be recorded");
                                }
                            }
                        }
                        Ok("failed".to_owned())
                    }
                    Err(e) => Err(e),
                },
                other => other,
            },
            Err(e) => Err(e.into()),
        };
        match started {
            Ok(reached) if reached == "active" => reached,
            // A unit left enabled after a failed start would bring Turbo
            // back on at the next boot, while the UI said it failed.
            failed => {
                manager.disable_unit_files(&[UNIT], false).await?;
                manager.reload().await?;
                match failed {
                    Ok(reached) => reached,
                    Err(e) => return Err(e),
                }
            }
        }
    } else {
        manager.stop_unit(UNIT, "replace").await?;
        let mut reached = settle(connection, "inactive").await?;
        if reached == "failed" {
            let failure = state(connection).await?.failure();
            info!(?failure, "falcond had failed; clearing it to turn it off");
            manager.reset_failed_unit(UNIT).await?;
            reached = settle(connection, "inactive").await?;
        }
        // Still stopping (falcond restoring a game's profile): disabling now
        // would report a failure for a switch that is happening. The caller
        // sees the state and can ask again.
        if reached != "inactive" {
            anyhow::bail!("{UNIT} is still {reached}; not disabled yet");
        }
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
    let Some(record) = Ownership::load()? else {
        return Ok(None);
    };
    let manager = manager(connection).await?;
    match record.unit_file_state.as_str() {
        "enabled" => {
            manager.enable_unit_files(&[UNIT], false, false).await?;
        }
        // Enabled until the next boot only, as it was found.
        "enabled-runtime" => {
            manager.enable_unit_files(&[UNIT], true, false).await?;
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
    // Only a note for the UI: failing to leave it changes nothing restored.
    let note = Release {
        released_at: bigame_core::unix_now(),
        restored: record.clone(),
    };
    if let Err(e) = serde_json::to_vec_pretty(&note)
        .map_err(anyhow::Error::from)
        .and_then(|json| crate::write_atomic(Path::new(RELEASE_RECORD), &json, 0o644))
    {
        warn!(error = %e, "could not record the release");
    }
    info!(?record, "released the game backend to its prior state");
    Ok(Some(record))
}

/// Ask a running falcond to re-read its configuration and profiles.
///
/// SIGHUP, which falcond handles as a reload. Not `systemctl
/// reload-or-restart`: falcond's unit has no `ExecReload`, so that restarts it
/// and tears down the profile of a game that is running. A stopped falcond is
/// left stopped: Turbo decides that, not a profile save.
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
/// start-up changed (`enable_performance_mode`, the global scheduler and
/// 3D V-Cache mode).
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
    fn the_release_record_round_trips() {
        let note = Release {
            released_at: 1_790_000_100,
            restored: Ownership {
                taken_at: 1_790_000_000,
                unit_file_state: "disabled".into(),
                was_active: false,
            },
        };
        let json = serde_json::to_string(&note).unwrap();
        assert_eq!(serde_json::from_str::<Release>(&json).unwrap(), note);
    }

    #[test]
    fn the_controlled_unit_is_fixed() {
        // Nothing a caller sends chooses the unit; it is a constant.
        assert_eq!(UNIT, "falcond.service");
        assert!(OWNERSHIP_RECORD.starts_with("/var/lib/bigame-mode/"));
        assert!(RELEASE_RECORD.starts_with("/var/lib/bigame-mode/"));
        // The UI reads both from the same places.
        assert_eq!(OWNERSHIP_RECORD, bigame_core::turbo::OWNERSHIP_RECORD);
        assert_eq!(RELEASE_RECORD, bigame_core::turbo::RELEASE_RECORD);
    }
}
