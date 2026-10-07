//! systemd's D-Bus API, as far as Big Game Mode needs it.
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
    /// Clear a unit's `failed` state and its start-rate counter, as
    /// `systemctl reset-failed` does.
    #[zbus(name = "ResetFailedUnit")]
    fn reset_failed_unit(&self, name: &str) -> zbus::Result<()>;
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
    /// How the service last ended: `success`, `exit-code`, `signal`,
    /// `core-dump`, `start-limit-hit`, `timeout`, …
    #[zbus(property, name = "Result")]
    fn result(&self) -> zbus::Result<String>;
    /// How the main process last ended: a `waitid()` code, `CLD_*`.
    #[zbus(property, name = "ExecMainCode")]
    fn exec_main_code(&self) -> zbus::Result<i32>;
    /// The main process's exit status, or the signal that ended it.
    #[zbus(property, name = "ExecMainStatus")]
    fn exec_main_status(&self) -> zbus::Result<i32>;
}

/// `waitid()` codes systemd reports in `ExecMainCode`.
const CLD_EXITED: i32 = 1;
const CLD_KILLED: i32 = 2;
const CLD_DUMPED: i32 = 3;

/// How a service's main process last ended, as systemd reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRun {
    /// `Result`: `success`, `exit-code`, `signal`, `core-dump`,
    /// `start-limit-hit`, …
    pub result: String,
    /// `ExecMainCode`: `CLD_EXITED` (1), `CLD_KILLED` (2), `CLD_DUMPED` (3),
    /// or 0 when no process has run.
    pub main_code: i32,
    /// `ExecMainStatus`: the exit status, or the signal for 2 and 3.
    pub main_status: i32,
    /// `NRestarts`: automatic restarts since the last start on request.
    pub restarts: u32,
}

/// Why a service last failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Ended by `SIGILL`: the processor met an instruction it does not have
    /// (a binary built for a newer CPU), or, rarely, corrupt code.
    IllegalInstruction,
    /// Ended by another signal (`SIGSEGV`, `SIGABRT`, …).
    Signal(i32),
    /// Exited with a non-zero status.
    ExitCode(i32),
    /// Something else systemd names in `Result` (`timeout`, `resources`, …).
    Other(String),
}

impl ServiceRun {
    /// Why the service last failed, or `None` if its last run did not fail.
    ///
    /// `ExecMainCode` and `ExecMainStatus` outlive the failure they describe:
    /// `systemctl reset-failed` sets `Result` back to `success` and leaves
    /// them. Only a `Result` other than `success` makes them current.
    #[must_use]
    pub fn failure(&self) -> Option<Failure> {
        if self.result.is_empty() || self.result == "success" {
            return None;
        }
        Some(match self.main_code {
            CLD_KILLED | CLD_DUMPED if self.main_status == libc::SIGILL => {
                Failure::IllegalInstruction
            }
            CLD_KILLED | CLD_DUMPED => Failure::Signal(self.main_status),
            CLD_EXITED if self.main_status != 0 => Failure::ExitCode(self.main_status),
            _ => Failure::Other(self.result.clone()),
        })
    }

    /// Whether systemd gave up restarting it: started too often too quickly.
    #[must_use]
    pub fn start_limit_hit(&self) -> bool {
        self.result == "start-limit-hit"
    }
}

impl Failure {
    /// The failure in systemd's own terms, for reports and logs:
    /// `SIGILL (illegal instruction)`, `SIGSEGV`, `exit status 1`, `timeout`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::IllegalInstruction => "SIGILL (illegal instruction)".to_owned(),
            Self::Signal(n) => signal_name(*n),
            Self::ExitCode(n) => format!("exit status {n}"),
            Self::Other(result) => result.clone(),
        }
    }
}

/// A signal's name, as systemd's journal prints it.
fn signal_name(signal: i32) -> String {
    match signal {
        libc::SIGILL => "SIGILL".into(),
        libc::SIGABRT => "SIGABRT".into(),
        libc::SIGBUS => "SIGBUS".into(),
        libc::SIGFPE => "SIGFPE".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGSEGV => "SIGSEGV".into(),
        libc::SIGTERM => "SIGTERM".into(),
        libc::SIGSYS => "SIGSYS".into(),
        n => format!("signal {n}"),
    }
}

/// A unit's state, as systemd reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitState {
    /// `enabled`, `disabled`, `masked`, `static`, … or `not-found`.
    pub unit_file_state: String,
    /// `active`, `inactive`, `failed`, `activating`, …
    pub active_state: String,
    /// How its main process last ended, for a service whose state could be
    /// read; `None` otherwise.
    pub service: Option<ServiceRun>,
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

    /// Whether the unit is in systemd's `failed` state.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.active_state == "failed"
    }

    /// Why the service last failed, if its last run failed.
    #[must_use]
    pub fn failure(&self) -> Option<Failure> {
        self.service.as_ref().and_then(ServiceRun::failure)
    }

    /// A state for a unit systemd does not know.
    fn not_found() -> Self {
        Self {
            unit_file_state: "not-found".into(),
            active_state: "inactive".into(),
            service: None,
        }
    }
}

/// Read a unit's state.
///
/// # Errors
/// Returns an error if systemd cannot be reached.
pub async fn unit_state(connection: &zbus::Connection, unit: &str) -> zbus::Result<UnitState> {
    let manager = ManagerProxy::new(connection).await?;
    let Ok(unit_file_state) = manager.get_unit_file_state(unit).await else {
        return Ok(UnitState::not_found());
    };
    let path = manager.load_unit(unit).await?;
    let proxy = UnitProxy::builder(connection)
        .path(path.clone())?
        .build()
        .await?;
    let active_state = proxy.active_state().await?;
    // Not every unit is a service, and the exit details only add to the
    // state: one that cannot be read leaves them out.
    let service = async {
        let s = ServiceProxy::builder(connection)
            .path(path)?
            .build()
            .await?;
        zbus::Result::Ok(ServiceRun {
            result: s.result().await?,
            main_code: s.exec_main_code().await?,
            main_status: s.exec_main_status().await?,
            restarts: s.n_restarts().await?,
        })
    }
    .await
    .ok();
    Ok(UnitState {
        unit_file_state,
        active_state,
        service,
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
            return Some(UnitState::not_found());
        };
        let path = manager.load_unit(unit).ok()?;
        let proxy = UnitProxyBlocking::builder(&self.connection)
            .path(path.clone())
            .ok()?
            .build()
            .ok()?;
        let active_state = proxy.active_state().ok()?;
        Some(UnitState {
            unit_file_state,
            active_state,
            service: self.service_run(path),
        })
    }

    /// How a service's main process last ended; `None` for a unit that is
    /// not a service or whose properties could not be read.
    fn service_run(&self, path: OwnedObjectPath) -> Option<ServiceRun> {
        let s = ServiceProxyBlocking::builder(&self.connection)
            .path(path)
            .ok()?
            .build()
            .ok()?;
        Some(ServiceRun {
            result: s.result().ok()?,
            main_code: s.exec_main_code().ok()?,
            main_status: s.exec_main_status().ok()?,
            restarts: s.n_restarts().ok()?,
        })
    }

    /// How many times systemd restarted a service on its own (after a crash
    /// or a kill) since it was last started on request, or `None` if the unit
    /// is not loaded or systemd could not be asked.
    #[must_use]
    pub fn restarts(&self, unit: &str) -> Option<u32> {
        self.unit_state(unit)?.service.map(|s| s.restarts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(result: &str, main_code: i32, main_status: i32) -> ServiceRun {
        ServiceRun {
            result: result.into(),
            main_code,
            main_status,
            restarts: 0,
        }
    }

    #[test]
    fn a_crash_loop_on_an_illegal_instruction_is_named() {
        // falcond 2.0.14 built for x86-64-v3 on a Sandy Bridge, as systemd
        // 261 reports it once it gives up: start-limit-hit, the last run
        // dumped core on signal 4.
        let r = ServiceRun {
            restarts: 5,
            ..run("start-limit-hit", CLD_DUMPED, libc::SIGILL)
        };
        assert_eq!(r.failure(), Some(Failure::IllegalInstruction));
        assert!(r.start_limit_hit());
        // And between two restarts of that loop.
        assert_eq!(
            run("core-dump", CLD_DUMPED, libc::SIGILL).failure(),
            Some(Failure::IllegalInstruction)
        );
        // Without a core dump (core dumps off) systemd says "killed".
        assert_eq!(
            run("signal", CLD_KILLED, libc::SIGILL).failure(),
            Some(Failure::IllegalInstruction)
        );
    }

    #[test]
    fn other_failures_keep_their_own_cause() {
        assert_eq!(
            run("core-dump", CLD_DUMPED, libc::SIGSEGV).failure(),
            Some(Failure::Signal(libc::SIGSEGV))
        );
        assert_eq!(
            run("exit-code", CLD_EXITED, 1).failure(),
            Some(Failure::ExitCode(1))
        );
        assert_eq!(
            run("timeout", 0, 0).failure(),
            Some(Failure::Other("timeout".into()))
        );
        assert!(!run("core-dump", CLD_DUMPED, libc::SIGSEGV).start_limit_hit());
    }

    #[test]
    fn a_reset_or_a_clean_stop_is_not_a_failure() {
        // After reset-failed, systemd 261 reports Result=success and keeps
        // the old ExecMainCode=3, ExecMainStatus=4: not a current failure.
        assert_eq!(run("success", CLD_DUMPED, libc::SIGILL).failure(), None);
        // A stop on request ends the process with SIGTERM.
        assert_eq!(run("success", CLD_KILLED, libc::SIGTERM).failure(), None);
        // A service that never ran.
        assert_eq!(run("success", 0, 0).failure(), None);
        assert_eq!(run("", 0, 0).failure(), None);
    }

    #[test]
    fn failures_read_as_systemd_prints_them() {
        assert_eq!(
            Failure::IllegalInstruction.describe(),
            "SIGILL (illegal instruction)"
        );
        assert_eq!(Failure::Signal(libc::SIGSEGV).describe(), "SIGSEGV");
        assert_eq!(Failure::Signal(64).describe(), "signal 64");
        assert_eq!(Failure::ExitCode(3).describe(), "exit status 3");
        assert_eq!(Failure::Other("timeout".into()).describe(), "timeout");
    }

    #[test]
    fn a_unit_state_answers_from_its_service() {
        let failed = UnitState {
            unit_file_state: "enabled".into(),
            active_state: "failed".into(),
            service: Some(run("start-limit-hit", CLD_DUMPED, libc::SIGILL)),
        };
        assert!(failed.is_failed() && !failed.is_active());
        assert_eq!(failed.failure(), Some(Failure::IllegalInstruction));
        let missing = UnitState::not_found();
        assert!(!missing.is_installed());
        assert_eq!(missing.failure(), None);
    }
}
