//! systemd's D-Bus API, as far as BiGame-mode needs it.
//!
//! Shared by the unprivileged UI, which only reads unit state (systemd allows
//! any local user to), and the root helper, which also starts, stops, enables
//! and disables the one unit it controls. Talking D-Bus rather than running
//! `systemctl` means no process is spawned and no argument reaches a shell.

use zbus::zvariant::OwnedObjectPath;

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait Manager {
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
pub trait Unit {
    #[zbus(property, name = "ActiveState")]
    fn active_state(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Service",
    default_service = "org.freedesktop.systemd1"
)]
pub trait Service {
    /// Automatic restarts since the service was last started on request.
    #[zbus(property, name = "NRestarts")]
    fn n_restarts(&self) -> zbus::Result<u32>;
}

/// A unit's state, as systemd reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitState {
    /// `enabled`, `disabled`, `masked`, `static`, … or `not-found`.
    pub unit_file_state: String,
    /// `active`, `inactive`, `failed`, `activating`, …
    pub active_state: String,
}

impl UnitState {
    /// Whether the unit is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active_state == "active"
    }

    /// Whether the unit exists at all.
    #[must_use]
    pub fn is_installed(&self) -> bool {
        self.unit_file_state != "not-found"
    }
}

/// Read a unit's state.
///
/// # Errors
/// Returns an error if systemd cannot be reached.
pub async fn unit_state(connection: &zbus::Connection, unit: &str) -> zbus::Result<UnitState> {
    let manager = ManagerProxy::new(connection).await?;
    let Ok(unit_file_state) = manager.get_unit_file_state(unit).await else {
        return Ok(UnitState {
            unit_file_state: "not-found".into(),
            active_state: "inactive".into(),
        });
    };
    let path = manager.load_unit(unit).await?;
    let proxy = UnitProxy::builder(connection).path(path)?.build().await?;
    Ok(UnitState {
        unit_file_state,
        active_state: proxy.active_state().await?,
    })
}

/// A reusable, blocking view of systemd for callers without a Tokio reactor
/// (the GTK main loop). Holds one bus connection for its lifetime rather than
/// opening one per question.
pub struct Reader {
    connection: zbus::blocking::Connection,
}

impl Reader {
    /// Connect to the system bus.
    #[must_use]
    pub fn system() -> Option<Self> {
        zbus::blocking::Connection::system()
            .ok()
            .map(|connection| Self { connection })
    }

    /// One connection for the whole process, opened on first use: every
    /// periodic reading shares it instead of connecting each time. A failed
    /// connect is not remembered, so a later call tries again.
    #[must_use]
    pub fn shared() -> Option<&'static Self> {
        static SHARED: std::sync::OnceLock<Reader> = std::sync::OnceLock::new();
        if let Some(reader) = SHARED.get() {
            return Some(reader);
        }
        let reader = Self::system()?;
        Some(SHARED.get_or_init(|| reader))
    }

    /// A unit's state, or `None` if systemd could not be asked.
    #[must_use]
    pub fn unit_state(&self, unit: &str) -> Option<UnitState> {
        let manager = ManagerProxyBlocking::new(&self.connection).ok()?;
        let Ok(unit_file_state) = manager.get_unit_file_state(unit) else {
            return Some(UnitState {
                unit_file_state: "not-found".into(),
                active_state: "inactive".into(),
            });
        };
        let path = manager.load_unit(unit).ok()?;
        let proxy = UnitProxyBlocking::builder(&self.connection)
            .path(path)
            .ok()?
            .build()
            .ok()?;
        Some(UnitState {
            unit_file_state,
            active_state: proxy.active_state().ok()?,
        })
    }

    /// How many times systemd restarted a service on its own (after a crash
    /// or a kill) since it was last started on request, or `None` if the unit
    /// is not loaded or systemd could not be asked.
    #[must_use]
    pub fn restarts(&self, unit: &str) -> Option<u32> {
        let manager = ManagerProxyBlocking::new(&self.connection).ok()?;
        manager.get_unit_file_state(unit).ok()?;
        let path = manager.load_unit(unit).ok()?;
        ServiceProxyBlocking::builder(&self.connection)
            .path(path)
            .ok()?
            .build()
            .ok()?
            .n_restarts()
            .ok()
    }
}
