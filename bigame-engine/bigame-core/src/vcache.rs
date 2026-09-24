//! AMD 3D V-Cache mode control via sysfs.
//!
//! Sysfs path: `/sys/bus/platform/drivers/amd_x3d_vcache/AMDI0101:00/amd_x3d_mode`

use std::path::Path;

use anyhow::Result;

/// Sysfs path for AMD X3D `VCache` mode.
const SYSFS_PATH: &str = "/sys/bus/platform/drivers/amd_x3d_vcache/AMDI0101:00/amd_x3d_mode";

/// Check whether `VCache` hardware is present.
#[must_use]
pub fn is_available() -> bool {
    Path::new(SYSFS_PATH).exists()
}

/// Read current `VCache` mode from sysfs.
///
/// Returns "frequency", "cache", or the raw sysfs string.
/// Returns `None` if hardware is unavailable.
#[must_use]
pub fn read_mode() -> Option<String> {
    std::fs::read_to_string(SYSFS_PATH)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Write `VCache` mode to sysfs via `DBus`.
///
/// # Errors
/// Returns error if `DBus` fails.
pub async fn set_mode(mode: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(mode, "frequency" | "cache"),
        "invalid VCache mode: {mode}"
    );

    let proxy = crate::dbus_client::daemon_proxy().await?;
    proxy.set_vcache_mode(mode).await?;

    Ok(())
}
