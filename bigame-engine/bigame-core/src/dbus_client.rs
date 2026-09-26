//! Typed client for the privileged BiGame-mode helper.
//!
//! Every method here crosses a privilege boundary. The helper authorizes each
//! call through Polkit and validates every argument on its own side — nothing
//! in this file may be treated as a security control, because an attacker
//! simply would not use it.

use zbus::proxy;

/// The root helper's D-Bus interface.
///
/// Method names are pinned explicitly and must match `bigame-daemon` exactly.
/// zbus would otherwise derive them from the Rust function names, which makes
/// the wire contract hostage to a refactor — and does not always produce the
/// obvious spelling (`set_vcache_mode` derives to `SetVcacheMode`).
#[proxy(
    interface = "com.biglinux.BiGameMode",
    default_service = "com.biglinux.BiGameMode",
    default_path = "/com/biglinux/BiGameMode"
)]
pub trait BiGameDaemon {
    /// Write a per-game profile into the falcond user profile directory.
    ///
    /// `name` must be a bare profile name; the helper rejects anything
    /// containing a path separator or `..`.
    #[zbus(name = "SaveProfile")]
    async fn save_profile(&self, name: &str, payload: &str) -> zbus::Result<()>;

    /// Delete a per-game profile by bare name.
    #[zbus(name = "DeleteProfile")]
    async fn delete_profile(&self, name: &str) -> zbus::Result<()>;

    /// Replace falcond's global configuration and ask it to reload.
    #[zbus(name = "ApplyFalcondConfig")]
    async fn apply_falcond_config(&self, config_payload: &str) -> zbus::Result<()>;

    /// Set the AMD 3D V-Cache mode.
    #[zbus(name = "SetVCacheMode")]
    async fn set_vcache_mode(&self, mode: &str) -> zbus::Result<()>;

    /// Set the CPU frequency governor on every online CPU.
    #[zbus(name = "SetCpuGovernor")]
    async fn set_cpu_governor(&self, governor: &str) -> zbus::Result<()>;

    /// Set the Energy Performance Preference on every online CPU.
    #[zbus(name = "SetCpuEpp")]
    async fn set_cpu_epp(&self, epp: &str) -> zbus::Result<()>;

    /// Set `power_dpm_force_performance_level` for one DRM card.
    #[zbus(name = "SetGpuDpmLevel")]
    async fn set_gpu_dpm_level(&self, card: &str, level: &str) -> zbus::Result<()>;

    /// Turn the game performance backend (falcond) on or off, persistently.
    /// Returns systemd's active state for the unit afterwards.
    #[zbus(name = "SetGameBackend")]
    async fn set_game_backend(&self, enabled: bool) -> zbus::Result<String>;

    /// Return falcond to its state before BiGame-mode first changed it.
    /// Returns whether there was anything to hand back.
    #[zbus(name = "ReleaseGameBackend")]
    async fn release_game_backend(&self) -> zbus::Result<bool>;

    /// Liveness probe.
    #[zbus(name = "Ping")]
    async fn ping(&self) -> zbus::Result<String>;
}

/// Why a call to the helper did not happen, when the reason is one the user
/// can do something about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperFailure {
    /// Polkit refused the call, or the user cancelled the password prompt.
    Refused,
    /// Nothing owns the helper's name on the system bus.
    NotRunning,
}

/// The [`HelperFailure`] one error in a chain stands for, if any.
#[must_use]
pub fn helper_failure(err: &(dyn std::error::Error + 'static)) -> Option<HelperFailure> {
    if let Some(e) = err.downcast_ref::<zbus::Error>() {
        return match e {
            // What a proxy call returns: the reply's error name, unparsed.
            zbus::Error::MethodError(name, _, _) => match name.as_str() {
                "org.freedesktop.DBus.Error.AccessDenied" => Some(HelperFailure::Refused),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner" => Some(HelperFailure::NotRunning),
                _ => None,
            },
            zbus::Error::FDO(fdo) => helper_failure(&**fdo),
            _ => None,
        };
    }
    match err.downcast_ref::<zbus::fdo::Error>()? {
        zbus::fdo::Error::AccessDenied(_) => Some(HelperFailure::Refused),
        zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_) => {
            Some(HelperFailure::NotRunning)
        }
        zbus::fdo::Error::ZBus(e) => helper_failure(e),
        _ => None,
    }
}

/// Connect to the helper on the system bus.
///
/// # Errors
/// Returns an error if the system bus or the service is unreachable.
pub async fn daemon_proxy() -> anyhow::Result<BiGameDaemonProxy<'static>> {
    let connection = zbus::Connection::system().await?;
    Ok(BiGameDaemonProxy::new(&connection).await?)
}

/// Blocking variant, for call sites that are not on a Tokio reactor.
///
/// # Errors
/// Returns an error if the system bus or the service is unreachable.
pub fn daemon_proxy_blocking() -> anyhow::Result<BiGameDaemonProxyBlocking<'static>> {
    let connection = zbus::blocking::Connection::system()?;
    Ok(BiGameDaemonProxyBlocking::new(&connection)?)
}
