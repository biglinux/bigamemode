//! System telemetry: CPU/GPU frequency, temperature, utilization from sysfs.

use anyhow::{Context, Result};
use tokio::fs;

/// CPU core telemetry snapshot.
#[derive(Debug, Clone)]
pub struct CpuSnapshot {
    /// Current frequency in MHz.
    pub freq_mhz: u64,
    /// Active governor.
    pub governor: String,
}

/// GPU telemetry snapshot.
#[derive(Debug, Clone)]
pub struct GpuSnapshot {
    /// Current GPU clock in MHz (if available).
    pub freq_mhz: Option<u64>,
    /// GPU temperature in °C (if available).
    pub temp_celsius: Option<f64>,
}

/// Read current CPU frequency for a given core.
///
/// # Errors
/// Returns error if sysfs path is unreadable or contains invalid data.
pub async fn cpu_freq_mhz(core: u32) -> Result<u64> {
    let path = format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_cur_freq");
    let content = fs::read_to_string(&path)
        .await
        .with_context(|| format!("read CPU freq: {path}"))?;
    let khz: u64 = content.trim().parse().context("parse CPU freq")?;
    Ok(khz / 1000)
}

/// Read current CPU governor for a given core.
///
/// # Errors
/// Returns error if sysfs path is unreadable.
pub async fn cpu_governor(core: u32) -> Result<String> {
    let path = format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_governor");
    let content = fs::read_to_string(&path)
        .await
        .with_context(|| format!("read governor: {path}"))?;
    Ok(content.trim().to_owned())
}

/// Collect full CPU snapshot for a given core.
///
/// # Errors
/// Returns error if any sysfs read fails.
pub async fn cpu_snapshot(core: u32) -> Result<CpuSnapshot> {
    let (freq, gov) = tokio::try_join!(cpu_freq_mhz(core), cpu_governor(core))?;
    Ok(CpuSnapshot {
        freq_mhz: freq,
        governor: gov,
    })
}

// ── GPU Telemetry ────────────────────────────────────────────────────────────

/// Collect a GPU telemetry snapshot for the card games render on.
///
/// Two audit findings are fixed here, and both were caused by walking
/// `/sys/class/drm` by hand:
///
/// * **TEL-01** — the old walk did `read_dir(...).ok()?` *inside* the loop, so
///   the first entry without a `device/hwmon` directory returned `None` from
///   the whole function. `/sys/class/drm` is full of such entries (connector
///   nodes like `card1-DP-1`, plus `renderD*` and `version`), and readdir order
///   is not stable, so GPU telemetry appeared and vanished between runs.
/// * **TEL-02** — even when it did complete, it returned the *first* readable
///   card. On a machine with an integrated and a discrete GPU that is the idle
///   integrated one, not the card actually rendering the game.
///
/// Both go away by asking [`crate::hardware`] which card matters and reading
/// only that one.
#[must_use]
pub async fn gpu_snapshot() -> GpuSnapshot {
    let hw = crate::hardware::Hardware::detect();
    gpu_snapshot_for(hw.render_gpu()).await
}

/// Collect a snapshot for a specific card.
///
/// Separated from [`gpu_snapshot`] so callers that already hold a
/// [`crate::hardware::Hardware`] do not re-scan sysfs on every sample.
#[must_use]
pub async fn gpu_snapshot_for(gpu: Option<&crate::hardware::Gpu>) -> GpuSnapshot {
    let Some(gpu) = gpu else {
        // No AMD/Intel DRM card identified — try NVIDIA's own tool before
        // giving up, since its cards are not always described through hwmon.
        return GpuSnapshot {
            freq_mhz: nvidia_smi_u64("clocks.current.graphics").await,
            temp_celsius: nvidia_smi_f64("temperature.gpu").await,
        };
    };

    // hwmon reports frequency in Hz and temperature in millidegrees.
    let freq_mhz = match gpu.hwmon_u64("freq1_input") {
        Some(hz) => Some(hz / 1_000_000),
        None => nvidia_smi_u64("clocks.current.graphics").await,
    };
    let temp_celsius = match gpu.hwmon_u64("temp1_input") {
        #[allow(clippy::cast_precision_loss)]
        Some(milli) => Some(milli as f64 / 1000.0),
        None => nvidia_smi_f64("temperature.gpu").await,
    };

    GpuSnapshot {
        freq_mhz,
        temp_celsius,
    }
}

/// Run `nvidia-smi --query-gpu=<field>` and return the trimmed first line.
///
/// Returns `None` when nvidia-smi is absent or fails, which is the normal case
/// on an AMD or Intel machine and must not be logged as an error.
async fn nvidia_smi(field: &str) -> Option<String> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args([
            &format!("--query-gpu={field}"),
            "--format=csv,noheader,nounits",
        ])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().next()?.trim().to_owned())
}

async fn nvidia_smi_u64(field: &str) -> Option<u64> {
    nvidia_smi(field).await?.parse().ok()
}

async fn nvidia_smi_f64(field: &str) -> Option<f64> {
    nvidia_smi(field).await?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_snapshot_clone() {
        let snap = CpuSnapshot {
            freq_mhz: 3600,
            governor: "performance".into(),
        };
        let cloned = snap.clone();
        assert_eq!(cloned.freq_mhz, 3600);
        assert_eq!(cloned.governor, "performance");
    }

    #[test]
    fn gpu_snapshot_clone() {
        let snap = GpuSnapshot {
            freq_mhz: Some(1800),
            temp_celsius: Some(72.5),
        };
        let cloned = snap.clone();
        assert_eq!(cloned.freq_mhz, Some(1800));
        assert_eq!(cloned.temp_celsius, Some(72.5));
    }

    #[test]
    fn gpu_snapshot_none_fields() {
        let snap = GpuSnapshot {
            freq_mhz: None,
            temp_celsius: None,
        };
        let cloned = snap.clone();
        assert!(cloned.freq_mhz.is_none());
        assert!(cloned.temp_celsius.is_none());
    }

    // Error paths — nonexistent CPU core → sysfs read fails

    #[tokio::test]
    async fn cpu_freq_nonexistent_core_errors() {
        assert!(cpu_freq_mhz(99999).await.is_err());
    }

    #[tokio::test]
    async fn cpu_governor_nonexistent_core_errors() {
        assert!(cpu_governor(99999).await.is_err());
    }

    #[tokio::test]
    async fn cpu_snapshot_nonexistent_core_errors() {
        assert!(cpu_snapshot(99999).await.is_err());
    }

    // ── GPU telemetry regressions ────────────────────────────────────────

    #[tokio::test]
    async fn gpu_snapshot_reads_the_render_gpu_on_this_machine() {
        let hw = crate::hardware::Hardware::detect();
        let Some(gpu) = hw.render_gpu() else {
            return; // headless CI — nothing to assert
        };
        let snap = gpu_snapshot_for(Some(gpu)).await;

        // TEL-01: the walk used to abort on the first connector node and
        // return nothing at all. If the card has a temperature attribute we
        // must now actually get a value back.
        if gpu.hwmon_u64("temp1_input").is_some() {
            let temp = snap.temp_celsius.expect("temperature should be readable");
            assert!(
                (0.0..=125.0).contains(&temp),
                "implausible temperature {temp}"
            );
        }
        if gpu.hwmon_u64("freq1_input").is_some() {
            let mhz = snap.freq_mhz.expect("clock should be readable");
            assert!(mhz < 10_000, "implausible clock {mhz} MHz");
        }
    }

    #[tokio::test]
    async fn gpu_snapshot_targets_the_discrete_card_not_the_first_one() {
        // TEL-02: on a dual-GPU box the old code sampled whichever card
        // readdir yielded first, which is the idle integrated one.
        let hw = crate::hardware::Hardware::detect();
        if hw.gpus.len() < 2 {
            return; // single-GPU host
        }
        let render = hw.render_gpu().expect("multi-GPU host must resolve one");
        if hw.gpus.iter().any(|g| g.discrete) {
            assert!(
                render.discrete,
                "render GPU should be the discrete card, got {}",
                render.card
            );
        }
    }

    #[tokio::test]
    async fn gpu_snapshot_without_a_card_does_not_panic() {
        // No DRM card and (almost certainly) no nvidia-smi: must yield empty
        // fields rather than failing.
        let snap = gpu_snapshot_for(None).await;
        let _ = snap.freq_mhz;
        let _ = snap.temp_celsius;
    }
}
