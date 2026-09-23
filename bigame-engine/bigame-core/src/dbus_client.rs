//! Typed client for the privileged BiGame-mode helper.
//!
//! Every method here crosses a privilege boundary. The helper authorizes each
//! call through Polkit and validates every argument on its own side — nothing
//! in this file may be treated as a security control, because an attacker
//! simply would not use it.

use zbus::proxy;

/// The root helper's D-Bus interface.
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
    async fn save_profile(&self, name: &str, payload: &str) -> zbus::Result<()>;

    /// Delete a per-game profile by bare name.
    async fn delete_profile(&self, name: &str) -> zbus::Result<()>;

    /// Replace falcond's global configuration and ask it to reload.
    async fn apply_falcond_config(&self, config_payload: &str) -> zbus::Result<()>;

    /// Set the AMD 3D V-Cache mode.
    async fn set_vcache_mode(&self, mode: &str) -> zbus::Result<()>;

    /// Set the CPU frequency governor on every online CPU.
    async fn set_cpu_governor(&self, governor: &str) -> zbus::Result<()>;

    /// Set the Energy Performance Preference on every online CPU.
    async fn set_cpu_epp(&self, epp: &str) -> zbus::Result<()>;

    /// Set `power_dpm_force_performance_level` for one DRM card.
    async fn set_gpu_dpm_level(&self, card: &str, level: &str) -> zbus::Result<()>;

    /// Liveness probe.
    async fn ping(&self) -> zbus::Result<String>;
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
