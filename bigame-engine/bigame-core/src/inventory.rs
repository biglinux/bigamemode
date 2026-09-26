//! The machine inventory that accompanies every benchmark result.
//!
//! A frame rate without the machine that produced it is not a measurement, it
//! is an anecdote. This module writes the `system.json` that sits beside each
//! result so that a number can later be checked against the hardware, kernel
//! and driver that produced it, and so that a cached result can be discarded
//! when the machine it describes is no longer the machine in front of us.
//!
//! **What is deliberately absent.** No hostname, no username, no home
//! directory, no IP address, no Steam library paths, no game list. The
//! inventory describes hardware and software versions, nothing that identifies
//! a person or a machine. That is what makes it safe to attach to a report the
//! user may want to share; anything identifying would have to be stripped by
//! hand, and a step that has to be remembered is a step that will be forgotten.

use std::path::Path;

use serde_json::{Value, json};

use crate::hardware::Hardware;

/// A fingerprint of the machine, for deciding whether a cached result applies.
///
/// Built from the parts that change a benchmark's outcome: the processor, the
/// rendering GPU, its driver, the kernel and the amount of memory. Two machines
/// with the same fingerprint should produce comparable numbers; a machine whose
/// fingerprint has changed since a result was cached should not trust it.
///
/// It is a content hash, not an identifier — it says nothing about *which*
/// machine this is, only what it is made of.
#[must_use]
pub fn fingerprint(hw: &Hardware) -> String {
    let gpu = hw.render_gpu();
    let parts = [
        hw.cpu.model.clone(),
        hw.cpu.logical_cpus.to_string(),
        gpu.map(|g| g.pci_id.clone()).unwrap_or_default(),
        gpu.map(|g| g.driver.clone()).unwrap_or_default(),
        hw.kernel.clone(),
        memory_total_kb()
            .map(|k| (k / 1024 / 1024).to_string())
            .unwrap_or_default(),
    ];
    // FNV-1a: short, stable across builds, and adequate for telling two
    // machines apart. Nothing here needs to resist an adversary.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in parts.join("\u{1f}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Build the inventory document.
#[must_use]
pub fn build(hw: &Hardware) -> Value {
    json!({
        "schema": "bigame.system/1",
        "fingerprint": fingerprint(hw),
        "cpu": {
            "model": hw.cpu.model,
            "vendor": format!("{:?}", hw.cpu.vendor),
            "physical_cores": hw.cpu.physical_cores,
            "logical_cpus": hw.cpu.logical_cpus,
            "smt": hw.cpu.smt,
            "hybrid": hw.cpu.hybrid,
            "scaling_driver": hw.cpu.scaling_driver,
            "available_governors": hw.cpu.available_governors,
            "current_governor": hw.cpu.current_governor,
            "available_epp": hw.cpu.available_epp,
            "current_epp": hw.cpu.current_epp,
            "amd_pstate_status": hw.cpu.amd_pstate_status,
            "vcache": hw.cpu.vcache.as_ref().map(|v| json!({
                "current_mode": v.current_mode,
            })),
        },
        "gpus": hw.gpus.iter().map(|g| json!({
            "card": g.card,
            "vendor": format!("{:?}", g.vendor),
            "pci_id": g.pci_id,
            "driver": g.driver,
            "discrete": g.discrete,
            "vram_total_bytes": g.vram_total_bytes,
            "connected_outputs": g.connected_outputs.len(),
            "dpm_level": g.dpm_level_path.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_owned()),
        })).collect::<Vec<_>>(),
        "render_gpu": hw.render_gpu().map(|g| g.card.clone()),
        "displays": hw.displays.iter().map(|d| json!({
            "max_mode": d.max_mode.map(|(w, h)| format!("{w}x{h}")),
        })).collect::<Vec<_>>(),
        "chassis": format!("{:?}", hw.chassis),
        "power_source": format!("{:?}", hw.power_source),
        "session": format!("{:?}", hw.session),
        "memory_total_kb": memory_total_kb(),
        "kernel": hw.kernel,
        "software": software_versions(),
        "scheduler": scheduler_state(),
        "note": "Contains no hostname, username, IP address or file path.",
    })
}

/// Total RAM, from `/proc/meminfo`.
fn memory_total_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Versions of the things that move a frame rate between kernel releases.
fn software_versions() -> Value {
    json!({
        "mesa": probe_version("glxinfo", &["-B"], "OpenGL core profile version")
            .or_else(|| probe_version("vulkaninfo", &["--summary"], "driverInfo")),
        "gamescope": probe_version("gamescope", &["--version"], ""),
        "mangohud": probe_version("mangohud", &["--version"], ""),
        "gamemode": probe_version("gamemoded", &["--version"], ""),
        "proton_ge": None::<String>,
    })
}

/// Run a program for its version string, returning `None` when it is absent.
///
/// A missing tool is a legitimate answer, not an error — the report should say
/// "not installed" rather than omit the field and leave a reader guessing.
fn probe_version(program: &str, args: &[&str], needle: &str) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = if needle.is_empty() {
        text.lines().next()?
    } else {
        text.lines().find(|l| l.contains(needle))?
    };
    Some(line.trim().to_owned())
}

/// Whether an alternative CPU scheduler is loaded.
fn scheduler_state() -> Value {
    let state = std::fs::read_to_string("/sys/kernel/sched_ext/state")
        .ok()
        .map(|s| s.trim().to_owned());
    json!({
        "sched_ext_supported": Path::new("/sys/kernel/sched_ext").exists(),
        "sched_ext_state": state,
        "sched_ext_active": std::fs::read_to_string("/sys/kernel/sched_ext/root/ops")
            .ok()
            .map(|s| s.trim().to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inventory_carries_nothing_identifying() {
        let hw = Hardware::detect();
        let text = serde_json::to_string(&build(&hw)).unwrap();

        // The things that must never reach a shareable report.
        for secret in [
            std::env::var("USER").unwrap_or_else(|_| "\u{0}unset".into()),
            std::env::var("HOME").unwrap_or_else(|_| "\u{0}unset".into()),
            std::fs::read_to_string("/etc/hostname")
                .map_or_else(|_| "\u{0}unset".into(), |s| s.trim().to_owned()),
        ] {
            if secret.starts_with('\u{0}') || secret.is_empty() {
                continue;
            }
            assert!(!text.contains(&secret), "the inventory leaked {secret:?}");
        }
    }

    #[test]
    fn the_fingerprint_is_stable_and_content_derived() {
        let hw = Hardware::detect();
        assert_eq!(fingerprint(&hw), fingerprint(&hw), "must not vary per call");
        assert_eq!(fingerprint(&hw).len(), 16);

        // A different kernel is a different machine for caching purposes.
        let mut other = Hardware::detect();
        other.kernel = format!("{}-modified", other.kernel);
        assert_ne!(fingerprint(&hw), fingerprint(&other));
    }

    #[test]
    fn a_missing_program_yields_none_rather_than_an_error() {
        assert_eq!(
            probe_version("bigame-no-such-program-exists", &["--version"], ""),
            None
        );
    }

    #[test]
    fn the_document_is_shaped_as_the_schema_says() {
        let hw = Hardware::detect();
        let doc = build(&hw);
        assert_eq!(doc["schema"], "bigame.system/1");
        assert!(doc["cpu"]["model"].is_string());
        assert!(doc["gpus"].is_array());
        assert!(doc["fingerprint"].as_str().is_some_and(|f| f.len() == 16));
        // Memory is read from /proc, which exists on any Linux this runs on.
        assert!(doc["memory_total_kb"].as_u64().is_some_and(|m| m > 0));
    }
}
