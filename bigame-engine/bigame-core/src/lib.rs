//! BiGame-mode core: system backend for gaming performance orchestration.
//!
//! Separates all system-level logic (D-Bus, sysfs, process management)
//! from the UI layer, enabling independent testing and headless operation.

pub mod benchmark;
pub mod booster;
pub mod capabilities;
pub mod config;
pub mod dbus;
pub mod dbus_client;
pub mod diagnostics;
pub mod fg;
pub mod game_settings;
pub mod games;
pub mod gamescope;
pub mod gpu_telemetry;
pub mod graphics;
pub mod hardware;
pub mod health;
pub mod inventory;
pub mod launcher;
pub mod library;
pub mod logs;
pub mod mangohud;
pub mod migration;
pub mod models;
pub mod network;
pub mod overview;
pub mod paths;
pub mod processes;
pub mod profiles;
pub mod recommend;
pub mod running;
pub mod sched;
pub mod status;
pub mod steam;
pub mod systemd;
pub mod turbo;
pub mod vcache;
pub mod video_config;
pub mod watch;

/// Seconds since the Unix epoch, as recorded in journals, manifests and
/// reports (0 if the clock is before 1970).
#[must_use]
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    /// Create a unique temp directory for tests.
    pub fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bigame_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test temp dir");
        dir
    }
}
