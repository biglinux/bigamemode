//! BiGame-mode core: system backend for gaming performance orchestration.
//!
//! Separates all system-level logic (D-Bus, sysfs, process management)
//! from the UI layer, enabling independent testing and headless operation.

pub mod benchmark;
pub mod booster;
pub mod capabilities;
pub mod config;
pub mod dbus;
pub mod diagnostics;
pub mod fg;
pub mod games;
pub mod gamescope;
pub mod governor;
pub mod hardware;
pub mod health;
pub mod inventory;
pub mod launcher;
pub mod logs;
pub mod migration;
pub mod models;
pub mod network;
pub mod polkit;
pub mod processes;
pub mod profiles;
pub mod recommend;
pub mod running;
pub mod sched;
pub mod status;
pub mod steam;
pub mod systemd;
pub mod telemetry;
pub mod turbo;
pub mod vcache;
pub mod video_config;
pub mod watch;

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
pub mod dbus_client;
