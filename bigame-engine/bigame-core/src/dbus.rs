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

/// Run a blocking zbus call from any context, async or not.
///
/// zbus's blocking API drives its own executor with `block_on`, and tokio
/// panics outright if that happens on a runtime worker: *"Cannot start a
/// runtime from within a runtime"*. Since these helpers are called both from
/// the GTK main thread (no runtime) and from the Booster engine (inside one),
/// the context has to be detected rather than assumed.
///
/// When a runtime is running, the call is moved to a plain OS thread and waited
/// on. That thread does not touch the runtime, so there is no deadlock, and a
/// D-Bus property read is short enough that the wait is not worth a more
/// elaborate mechanism.
fn blocking_dbus<T, F>(f: F) -> Option<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_err() {
        return Some(f());
    }
    std::thread::Builder::new()
        .name("bigame-dbus-sync".into())
        .spawn(f)
        .ok()?
        .join()
        .ok()
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
    blocking_dbus(|| {
        let conn = system_conn()?;
        PowerProfilesProxyBlocking::new(conn)
            .ok()
            .and_then(|p| p.active_profile().ok())
    })
    .flatten()
}

/// Set power profile (blocking).
///
/// Valid values are whatever [`power_profiles_available`] reports — typically
/// "balanced", "performance", "power-saver". Returns `false` when the write did
/// not happen, which callers must treat as a failure rather than ignoring.
#[must_use]
pub fn power_profile_set(profile: &str) -> bool {
    let profile = profile.to_owned();
    blocking_dbus(move || {
        let Some(conn) = system_conn() else {
            return false;
        };
        PowerProfilesProxyBlocking::new(conn)
            .ok()
            .and_then(|p| p.set_active_profile(&profile).ok())
            .is_some()
    })
    .unwrap_or(false)
}

/// Profile names power-profiles-daemon offers on this machine.
///
/// Returns an empty list when the daemon is unreachable. Never assume the usual
/// three exist: on some platforms `performance` is absent entirely.
#[must_use]
pub fn power_profiles_available() -> Vec<String> {
    blocking_dbus(|| {
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
    })
    .unwrap_or_default()
}

/// Whether a well-known name currently has an owner on the system bus.
///
/// Used to tell "the service is installed but not running" apart from "the
/// feature does not exist here" — the distinction audit finding SCX-02 needed.
#[must_use]
pub fn system_service_running(name: &str) -> bool {
    let name = name.to_owned();
    blocking_dbus(move || {
        let Some(conn) = system_conn() else {
            return false;
        };
        let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(conn) else {
            return false;
        };
        let Ok(bus_name) = zbus::names::BusName::try_from(name.as_str()) else {
            return false;
        };
        proxy.name_has_owner(bus_name).unwrap_or(false)
    })
    .unwrap_or(false)
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

        // Audit DBUS-01: this loop used to re-read the status file every
        // 500 ms for the life of the process — two wakeups a second, forever,
        // in an application whose purpose is to stay out of a game's way.
        //
        // falcond owns no D-Bus name to subscribe to, so the file really is the
        // only channel; but watching it costs nothing while nothing happens.
        // The watcher thread blocks in the kernel and only speaks when the
        // contents actually change.
        let path = std::path::Path::new(crate::status::STATUS_PATH);
        let Some(mut changes) = crate::watch::watch_file(path) else {
            tracing::warn!(
                "could not watch {}; falcond status will not be broadcast",
                crate::status::STATUS_PATH
            );
            return Ok(());
        };

        loop {
            // The watcher is a blocking thread, so receiving is moved off the
            // reactor rather than blocking it.
            let received =
                tokio::task::spawn_blocking(move || changes.recv().ok().map(|c| (c, changes)))
                    .await;
            let Ok(Some((content, returned))) = received else {
                tracing::debug!("falcond status watcher stopped");
                return Ok(());
            };
            changes = returned;

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
