//! BiGame-mode core: system backend for gaming performance orchestration.
//!
//! Separates all system-level logic (D-Bus, sysfs, process management)
//! from the UI layer, enabling independent testing and headless operation.

pub mod benchmark;
pub mod booster;
pub mod capabilities;
pub mod config;
pub mod dbus;
pub mod fg;
pub mod games;
pub mod gamescope;
pub mod governor;
pub mod hardware;
pub mod launcher;
pub mod models;
pub mod network;
pub mod polkit;
pub mod profiles;
pub mod sched;
pub mod status;
pub mod steam;
pub mod telemetry;
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
