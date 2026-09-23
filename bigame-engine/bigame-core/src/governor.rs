//! CPU frequency governor management via sysfs.

use anyhow::{Context, Result};
use tokio::fs;

/// Set the CPU frequency governor for all online cores via `DBus`.
///
/// # Errors
/// Returns error if the governor name is invalid or `DBus` fails.
pub async fn set(governor: &str) -> Result<()> {
    // Strict validation: [a-zA-Z0-9_] only — prevents injection.
    anyhow::ensure!(
        !governor.is_empty() && governor.chars().all(|c| c.is_alphanumeric() || c == '_'),
        "invalid governor name: {governor}"
    );

    let proxy = crate::dbus_client::daemon_proxy().await?;
    proxy.set_cpu_governor(governor).await?;

    Ok(())
}

/// List available CPU governors from sysfs.
///
/// # Errors
/// Returns error if sysfs path is unreadable.
pub async fn available() -> Result<Vec<String>> {
    let content =
        fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")
            .await
            .context("read available governors")?;
    Ok(content.split_whitespace().map(String::from).collect())
}

/// Current governor for a specific CPU core.
///
/// # Errors
/// Returns error if sysfs path is unreadable.
pub async fn current(core: u32) -> Result<String> {
    let path = format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_governor");
    let content = fs::read_to_string(&path)
        .await
        .with_context(|| format!("read governor: {path}"))?;
    Ok(content.trim().to_owned())
}
