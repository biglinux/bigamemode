//! D-Bus integration with `PowerProfiles` and falcond status daemons.

#![allow(clippy::missing_errors_doc)]

use std::sync::OnceLock;

/// Cached system bus connection (for `PowerProfiles`).
static SYSTEM_CONN: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();

fn system_conn() -> Option<&'static zbus::blocking::Connection> {
    SYSTEM_CONN
        .get_or_init(|| zbus::blocking::Connection::system().ok())
        .as_ref()
}

/// Check if falcond service is running by looking for its status file.
#[must_use]
pub fn falcond_is_running() -> bool {
    std::path::Path::new(crate::status::STATUS_PATH).exists()
}

// ── PowerProfiles ───────────────────────────────────────────────────────────

/// Proxy for `net.hadess.PowerProfiles` (power-profiles-daemon).
#[zbus::proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    /// Current active power profile.
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;

    /// Set the active power profile.
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;

    /// All profiles the daemon offers, as a list of property dictionaries.
    #[zbus(property)]
    fn profiles(
        &self,
    ) -> zbus::Result<Vec<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>>;
}

/// Get current power profile (blocking).
///
/// Returns "balanced", "performance", or "power-saver".
/// Returns `None` if daemon is unavailable.
#[must_use]
pub fn power_profile_get() -> Option<String> {
    let conn = system_conn()?;
    PowerProfilesProxyBlocking::new(conn)
        .ok()
        .and_then(|p| p.active_profile().ok())
}

/// Set power profile (blocking).
///
/// Valid values are whatever [`power_profiles_available`] reports — typically
/// "balanced", "performance", "power-saver". Returns `false` when the write did
/// not happen, which callers must treat as a failure rather than ignoring.
#[must_use]
pub fn power_profile_set(profile: &str) -> bool {
    let Some(conn) = system_conn() else {
        return false;
    };
    PowerProfilesProxyBlocking::new(conn)
        .ok()
        .and_then(|p| p.set_active_profile(profile).ok())
        .is_some()
}

/// Profile names power-profiles-daemon offers on this machine.
///
/// Returns an empty list when the daemon is unreachable. Never assume the usual
/// three exist: on some platforms `performance` is absent entirely.
#[must_use]
pub fn power_profiles_available() -> Vec<String> {
    let Some(conn) = system_conn() else {
        return Vec::new();
    };
    let Ok(proxy) = PowerProfilesProxyBlocking::new(conn) else {
        return Vec::new();
    };
    let Ok(profiles) = proxy.profiles() else {
        return Vec::new();
    };
    profiles
        .iter()
        .filter_map(|dict| {
            let v = dict.get("Profile")?;
            <&str>::try_from(v).ok().map(str::to_owned)
        })
        .collect()
}

/// Whether a well-known name currently has an owner on the system bus.
///
/// Used to tell "the service is installed but not running" apart from "the
/// feature does not exist here" — the distinction audit finding SCX-02 needed.
#[must_use]
pub fn system_service_running(name: &str) -> bool {
    let Some(conn) = system_conn() else {
        return false;
    };
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(conn) else {
        return false;
    };
    let Ok(bus_name) = zbus::names::BusName::try_from(name) else {
        return false;
    };
    proxy.name_has_owner(bus_name).unwrap_or(false)
}

// ── Falcond status D-Bus service ────────────────────────────────────────────

/// D-Bus service that broadcasts `falcond` status changes on the session bus.
///
/// Bus name   : `com.biglinux.BiGameMode1` (session)
/// Object path: `/com/biglinux/BiGameMode/Falcond`
/// Interface  : `com.biglinux.BiGameMode.Falcond`
///
/// External tools (scripts, shell widgets, monitors) can subscribe to the
/// `StatusChanged` signal instead of polling `/tmp/falcond_status` directly.
pub mod service {
    use zbus::object_server::SignalEmitter;

    const BUS_NAME: &str = "com.biglinux.BiGameMode1";
    const OBJECT_PATH: &str = "/com/biglinux/BiGameMode/Falcond";
    /// How often the service polls the status file for changes.
    const POLL_MS: u64 = 500;

    struct FalcondIface;

    #[zbus::interface(name = "com.biglinux.BiGameMode.Falcond")]
    impl FalcondIface {
        /// Return current falcond status (raw key-value text).
        #[allow(clippy::unused_self)] // zbus interface methods require &self
        fn get_status(&self) -> String {
            std::fs::read_to_string(crate::status::STATUS_PATH).unwrap_or_default()
        }

        /// Emitted whenever `/tmp/falcond_status` changes.
        #[zbus(signal)]
        async fn status_changed(ctx: &SignalEmitter<'_>, content: &str) -> zbus::Result<()>;
    }

    /// Async service loop: register on the session bus, poll the file, emit signals.
    async fn run() -> zbus::Result<()> {
        let conn = zbus::connection::Builder::session()?
            .name(BUS_NAME)?
            .serve_at(OBJECT_PATH, FalcondIface)?
            .build()
            .await?;

        tracing::info!("falcond D-Bus status service registered as {BUS_NAME}");

        let mut last = String::new();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(POLL_MS)).await;

            let Ok(content) = tokio::fs::read_to_string(crate::status::STATUS_PATH).await else {
                continue;
            };

            if content == last {
                continue;
            }
            last = content.clone();

            let iface_ref = conn
                .object_server()
                .interface::<_, FalcondIface>(OBJECT_PATH)
                .await;
            let Ok(iface) = iface_ref else {
                continue;
            };
            FalcondIface::status_changed(iface.signal_emitter(), &content)
                .await
                .ok();
        }
    }

    /// Spawn the falcond D-Bus status service in a background thread.
    ///
    /// Non-blocking: returns immediately. The service runs for the process lifetime.
    /// Silently exits if the session bus is unavailable (headless/tty environments).
    pub fn start() {
        std::thread::Builder::new()
            .name("bigame-dbus-service".into())
            .spawn(|| {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("D-Bus service: failed to create tokio runtime: {e}");
                        return;
                    }
                };
                rt.block_on(async {
                    if let Err(e) = run().await {
                        // Not an error in headless/tty environments where no session bus exists.
                        tracing::debug!("falcond D-Bus status service exited: {e}");
                    }
                });
            })
            .ok(); // silently ignore thread spawn failure
    }
}
