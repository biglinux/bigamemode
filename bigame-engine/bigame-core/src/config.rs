//! Read/write falcond configuration (`/etc/falcond/config.conf`).
//!
//! Config file is TOML. Writing requires root (pkexec).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// falcond's configuration file.
///
/// This is `config.conf`. It is **not** `falcond.conf`, which is what this
/// project used to read and write — a path falcond never opens, so every global
/// setting the Tuning page offered silently did nothing while reporting
/// success (audit CFG-01).
///
/// Verified against falcond 2.0.2: its binary contains the string
/// `/etc/falcond/config.conf` and no other configuration path, and
/// `/etc/falcond/` on a stock install contains exactly that one file.
pub const CONFIG_PATH: &str = "/etc/falcond/config.conf";

/// Falcond daemon configuration (mirrors Zig `Config` struct).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalcondConfig {
    /// Enable performance mode switching for matched games.
    #[serde(default = "default_true")]
    pub enable_performance_mode: bool,
    /// Global sched-ext scheduler (none, bpfland, lavd, rusty, flash).
    #[serde(default)]
    pub scx_sched: String,
    /// Scheduler mode (default, gaming, power, latency, server).
    #[serde(default = "default_mode")]
    pub scx_sched_props: String,
    /// `VCache` mode (none, cache, freq).
    #[serde(default)]
    pub vcache_mode: String,
    /// Profile mode (none, handheld, htpc).
    #[serde(default)]
    pub profile_mode: String,
    /// /proc scan interval in milliseconds.
    #[serde(default = "default_poll")]
    pub poll_interval_ms: u32,
    /// Process names excluded from performance-mode activation.
    /// Preserved on read-modify-write to avoid clobbering user config.
    #[serde(default)]
    pub system_processes: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_mode() -> String {
    "default".into()
}

fn default_poll() -> u32 {
    9000
}

impl Default for FalcondConfig {
    fn default() -> Self {
        Self {
            enable_performance_mode: true,
            scx_sched: "none".into(),
            scx_sched_props: "default".into(),
            vcache_mode: "none".into(),
            profile_mode: "none".into(),
            poll_interval_ms: 9000,
            system_processes: Vec::new(),
        }
    }
}

/// Read falcond config from disk.
///
/// # Errors
/// Returns error if file is unreadable or contains invalid TOML.
pub fn read() -> Result<FalcondConfig> {
    read_from(Path::new(CONFIG_PATH))
}

/// Read falcond config from a specific path.
///
/// Supports both `otter_conf` (bare identifiers) and TOML (quoted strings) formats.
///
/// # Errors
/// Returns error if file is unreadable or unparseable.
pub fn read_from(path: &Path) -> Result<FalcondConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read falcond config: {}", path.display()))?;
    // Try TOML first (backwards compat with old configs)
    if let Ok(cfg) = toml::from_str::<FalcondConfig>(&content) {
        return Ok(cfg);
    }
    // Fall back to otter_conf key=value parsing (bare identifiers)
    Ok(parse_otter_conf(&content))
}

/// Parse `otter_conf` format: `key = value` with bare identifiers for enums.
fn parse_otter_conf(content: &str) -> FalcondConfig {
    let mut cfg = FalcondConfig::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        // Strip optional quotes (handle both `"none"` and `none`)
        let val = val.trim_matches('"');
        match key {
            "enable_performance_mode" => cfg.enable_performance_mode = val == "true",
            "scx_sched" => cfg.scx_sched = val.to_string(),
            "scx_sched_props" => cfg.scx_sched_props = val.to_string(),
            "vcache_mode" => cfg.vcache_mode = val.to_string(),
            "profile_mode" => cfg.profile_mode = val.to_string(),
            "poll_interval_ms" => cfg.poll_interval_ms = val.parse().unwrap_or(9000),
            "system_processes" => {
                // Parse ["str1", "str2"] array syntax
                let inner = val.trim_start_matches('[').trim_end_matches(']');
                if !inner.is_empty() {
                    cfg.system_processes = inner
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            _ => {} // Ignore unknown fields
        }
    }
    cfg
}

/// [`write`] for callers without a Tokio reactor — the GTK main loop, or a
/// `gio::spawn_blocking` worker. zbus here runs on Tokio, so its async calls
/// cannot simply be awaited from a `GLib` future.
///
/// # Errors
/// Returns an error if the runtime cannot be built or the write fails.
pub fn write_blocking(config: &FalcondConfig) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(write(config))
}

/// Write falcond config via `DBus`.
///
/// Uses `DBus` to write as root, then sends `SIGHUP` to falcond
/// so it reloads the config without restart.
///
/// # Errors
/// Returns error if serialization or `DBus` fails.
pub async fn write(config: &FalcondConfig) -> Result<()> {
    write_to(config, Path::new(CONFIG_PATH)).await
}

/// Serialize config to `otter_conf` format (bare identifiers for enums, no TOML quoting).
///
/// `otter_conf` expects: `key = bare_value` for enums/bools/ints,
/// and `key = "quoted"` only for string slices.
fn serialize_otter_conf(config: &FalcondConfig) -> String {
    let mut out = String::new();
    // Bool: bare true/false
    let _ = writeln!(
        out,
        "enable_performance_mode = {}",
        config.enable_performance_mode
    );
    // Enum fields: bare identifiers (no quotes)
    let _ = writeln!(out, "scx_sched = {}", config.scx_sched);
    let _ = writeln!(out, "scx_sched_props = {}", config.scx_sched_props);
    let _ = writeln!(out, "vcache_mode = {}", config.vcache_mode);
    let _ = writeln!(out, "profile_mode = {}", config.profile_mode);
    // Integer: bare number
    let _ = writeln!(out, "poll_interval_ms = {}", config.poll_interval_ms);
    // String array: ["quoted", "strings"]
    if !config.system_processes.is_empty() {
        let quoted: Vec<String> = config
            .system_processes
            .iter()
            .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        let _ = writeln!(out, "system_processes = [{}]", quoted.join(", "));
    }
    out
}

/// Write falcond config via `DBus`.
///
/// # Errors
/// Returns error if serialization or `DBus` fails.
pub async fn write_to(config: &FalcondConfig, _path: &Path) -> Result<()> {
    let content = serialize_otter_conf(config);

    let proxy = crate::dbus_client::daemon_proxy().await?;
    proxy.apply_falcond_config(&content).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let cfg = FalcondConfig::default();
        assert!(cfg.enable_performance_mode);
        assert_eq!(cfg.scx_sched, "none");
        assert_eq!(cfg.scx_sched_props, "default");
        assert_eq!(cfg.vcache_mode, "none");
        assert_eq!(cfg.profile_mode, "none");
        assert_eq!(cfg.poll_interval_ms, 9000);
    }

    #[test]
    fn round_trip_serialization() {
        let cfg = FalcondConfig {
            enable_performance_mode: false,
            scx_sched: "bpfland".into(),
            scx_sched_props: "gaming".into(),
            vcache_mode: "cache".into(),
            profile_mode: "handheld".into(),
            poll_interval_ms: 5000,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: FalcondConfig = toml::from_str(&toml_str).unwrap();
        assert!(!parsed.enable_performance_mode);
        assert_eq!(parsed.scx_sched, "bpfland");
        assert_eq!(parsed.scx_sched_props, "gaming");
        assert_eq!(parsed.vcache_mode, "cache");
        assert_eq!(parsed.poll_interval_ms, 5000);
    }

    #[test]
    fn read_from_file() {
        let dir = std::env::temp_dir().join("bigame_test_config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.conf");
        let content = r#"
enable_performance_mode = true
scx_sched = "lavd"
scx_sched_props = "latency"
vcache_mode = "freq"
profile_mode = "htpc"
poll_interval_ms = 3000
"#;
        std::fs::write(&path, content).unwrap();
        let cfg = read_from(&path).unwrap();
        assert!(cfg.enable_performance_mode);
        assert_eq!(cfg.scx_sched, "lavd");
        assert_eq!(cfg.scx_sched_props, "latency");
        assert_eq!(cfg.vcache_mode, "freq");
        assert_eq!(cfg.profile_mode, "htpc");
        assert_eq!(cfg.poll_interval_ms, 3000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let content = "scx_sched = \"rusty\"\n";
        let cfg: FalcondConfig = toml::from_str(content).unwrap();
        assert!(cfg.enable_performance_mode); // default_true
        assert_eq!(cfg.scx_sched, "rusty");
        assert_eq!(cfg.scx_sched_props, "default"); // default_mode
        assert_eq!(cfg.poll_interval_ms, 9000); // default_poll
    }
}
