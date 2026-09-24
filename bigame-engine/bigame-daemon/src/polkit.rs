//! Polkit authorization for the privileged helper.
//!
//! Every method on the helper's interface must pass through [`check`] before it
//! touches the system. The bus policy lets every local uid call in, so the
//! actions in the `.policy` file protect nothing unless this code consults
//! them.
//!
//! The caller is identified by its **unique bus name**, not by a PID. A PID can
//! be recycled between the moment a message is sent and the moment it is
//! checked; a unique bus name cannot, because the bus itself guarantees it is
//! never reused for the lifetime of the bus. This is the identification scheme
//! Polkit documents for D-Bus services precisely to close that race.

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

/// Polkit action identifiers. These must match `data/*.policy` exactly.
pub mod actions {
    /// Set the CPU frequency governor or energy preference.
    pub const SET_CPU: &str = "com.biglinux.bigamemode.set-cpu";
    /// Set a GPU power/DPM level.
    pub const SET_GPU: &str = "com.biglinux.bigamemode.set-gpu";
    /// Set the AMD 3D V-Cache mode.
    pub const SET_VCACHE: &str = "com.biglinux.bigamemode.set-vcache";
    /// Write falcond's global configuration.
    pub const WRITE_CONFIG: &str = "com.biglinux.bigamemode.write-config";
    /// Create or delete per-game profiles.
    pub const MANAGE_PROFILES: &str = "com.biglinux.bigamemode.manage-profiles";
    /// Turn the game performance backend (falcond) on or off.
    pub const CONTROL_BACKEND: &str = "com.biglinux.bigamemode.control-backend";
}

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    #[zbus(name = "CheckAuthorization")]
    #[allow(clippy::type_complexity)]
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, OwnedValue>),
        action_id: &str,
        details: HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

/// Allow Polkit to prompt the user. The helper is invoked from an interactive
/// desktop application, so an authentication dialog is the expected behaviour
/// rather than an outright denial.
const ALLOW_USER_INTERACTION: u32 = 1;

/// Authorize `sender` for `action`, or return the D-Bus error to reply with.
///
/// # Errors
/// Returns [`zbus::fdo::Error::AccessDenied`] when the caller is not
/// authorized, when the caller could not be identified, or when Polkit itself
/// is unreachable. **Failing closed is deliberate**: a helper that grants root
/// because its authorization service is down is worse than one that stops
/// working.
pub async fn check(
    connection: &zbus::Connection,
    sender: Option<&zbus::names::UniqueName<'_>>,
    action: &str,
) -> Result<(), zbus::fdo::Error> {
    let Some(sender) = sender else {
        tracing::warn!(action, "refusing a request with no identifiable sender");
        return Err(zbus::fdo::Error::AccessDenied(
            "caller could not be identified".into(),
        ));
    };

    let authority = AuthorityProxy::new(connection).await.map_err(|e| {
        tracing::error!(action, error = %e, "polkit authority is unreachable");
        zbus::fdo::Error::AccessDenied("authorization service unavailable".into())
    })?;

    let name = OwnedValue::from(zbus::zvariant::Str::from(sender.as_str().to_owned()));
    let subject_details: HashMap<&str, OwnedValue> = HashMap::from([("name", name)]);

    let (authorized, _challenge, _details) = authority
        .check_authorization(
            &("system-bus-name", subject_details),
            action,
            HashMap::new(),
            ALLOW_USER_INTERACTION,
            "",
        )
        .await
        .map_err(|e| {
            tracing::error!(action, error = %e, "polkit check failed");
            zbus::fdo::Error::AccessDenied("authorization check failed".into())
        })?;

    if authorized {
        tracing::debug!(action, caller = %sender, "authorized");
        Ok(())
    } else {
        tracing::warn!(action, caller = %sender, "denied by policy");
        Err(zbus::fdo::Error::AccessDenied(format!(
            "not authorized for {action}"
        )))
    }
}
