//! Knobs — the individual pieces of system state Booster Mode may change.
//!
//! A knob is the smallest unit the engine understands, and it is deliberately
//! narrow: it can report its current value, write a new one, and say who is
//! allowed to write it. Everything interesting — planning, verification,
//! rollback — is built on top of those three operations, which is what makes
//! "did the change actually take effect?" a property of the architecture rather
//! than something each feature has to remember to do.
//!
//! Knobs never decide *whether* a change is a good idea. That is the planner's
//! job.

use std::fmt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::graphics::text::N_;

/// Identifies one piece of system state.
///
/// Serialized into the crash-recovery journal, so the string forms are a
/// stability contract — renaming a variant orphans journals written by an
/// older build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Knob {
    /// power-profiles-daemon active profile.
    PowerProfile,
    /// CPU frequency governor, applied to every online CPU.
    CpuGovernor,
    /// Energy Performance Preference (`amd-pstate-epp` / `intel_pstate`).
    CpuEpp,
    /// `power_dpm_force_performance_level` for one DRM card.
    GpuDpmLevel {
        /// DRM node name, e.g. `card1`.
        card: String,
    },
    /// AMD 3D V-Cache mode.
    VCacheMode,
}

impl Knob {
    /// Stable identifier used in plans, reports and the journal.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            Self::PowerProfile => "power_profile".into(),
            Self::CpuGovernor => "cpu_governor".into(),
            Self::CpuEpp => "cpu_epp".into(),
            Self::GpuDpmLevel { card } => format!("gpu_dpm_level:{card}"),
            Self::VCacheMode => "vcache_mode".into(),
        }
    }

    /// Short human label for the UI.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::PowerProfile => N_("Power profile").into(),
            Self::CpuGovernor => N_("CPU governor").into(),
            Self::CpuEpp => N_("CPU energy preference").into(),
            Self::GpuDpmLevel { card } => N_("GPU power level (%s)").replace("%s", card),
            Self::VCacheMode => N_("3D V-Cache mode").into(),
        }
    }

    /// Read the current value, or `None` when this knob does not exist here.
    ///
    /// Reads always go straight to the source of truth — sysfs or the owning
    /// daemon — never to a cache, because the whole point of verification is to
    /// observe what the system really did.
    #[must_use]
    pub fn read(&self) -> Option<String> {
        match self {
            Self::PowerProfile => crate::dbus::power_profile_get(),
            Self::CpuGovernor => read_sysfs(&cpu_attr_path(0, "scaling_governor")),
            Self::CpuEpp => read_sysfs(&cpu_attr_path(0, "energy_performance_preference")),
            Self::GpuDpmLevel { card } => read_sysfs(&PathBuf::from(format!(
                "/sys/class/drm/{card}/device/power_dpm_force_performance_level"
            ))),
            Self::VCacheMode => crate::hardware::Hardware::detect().cpu.vcache?.current_mode,
        }
    }

    /// Values this knob will accept on this machine.
    ///
    /// Empty means "this knob does not exist here", which the planner treats as
    /// grounds to skip rather than to fail.
    #[must_use]
    pub fn allowed_values(&self) -> Vec<String> {
        match self {
            Self::PowerProfile => crate::dbus::power_profiles_available(),
            Self::CpuGovernor => split_sysfs(&cpu_attr_path(0, "scaling_available_governors")),
            Self::CpuEpp => split_sysfs(&cpu_attr_path(
                0,
                "energy_performance_available_preferences",
            )),
            Self::GpuDpmLevel { card } => {
                // amdgpu does not enumerate these; the set is fixed by the driver.
                if PathBuf::from(format!(
                    "/sys/class/drm/{card}/device/power_dpm_force_performance_level"
                ))
                .exists()
                {
                    [
                        "auto",
                        "low",
                        "high",
                        "manual",
                        "profile_standard",
                        "profile_min_sclk",
                        "profile_min_mclk",
                        "profile_peak",
                    ]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
                } else {
                    Vec::new()
                }
            }
            Self::VCacheMode => {
                if crate::hardware::Hardware::detect().cpu.vcache.is_some() {
                    vec!["frequency".into(), "cache".into()]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Whether `value` is one this knob accepts.
    #[must_use]
    pub fn accepts(&self, value: &str) -> bool {
        self.allowed_values().iter().any(|v| v == value)
    }

    /// Write `value`, routing through whichever mechanism owns this knob.
    ///
    /// Returns `Ok(())` only when the write was *attempted successfully*. It
    /// does **not** mean the system now holds that value — that is what
    /// [`Knob::verify`] is for, and the two are kept separate on purpose: an
    /// unchecked write is how a control reports success while the system stays
    /// unchanged.
    ///
    /// # Errors
    /// Returns an error if the value is not accepted here, or if the write
    /// mechanism fails.
    pub async fn write(&self, value: &str) -> Result<()> {
        anyhow::ensure!(
            self.accepts(value),
            "{} does not accept {value:?} on this machine (accepted: {:?})",
            self.title(),
            self.allowed_values()
        );
        match self {
            Self::PowerProfile => {
                // power-profiles-daemon is driven through zbus's blocking API,
                // which must not be called on a runtime worker: it would block
                // the reactor the rest of this function depends on.
                let v = value.to_owned();
                let ok = tokio::task::spawn_blocking(move || crate::dbus::power_profile_set(&v))
                    .await
                    .context("power profile write task")?;
                anyhow::ensure!(ok, "power-profiles-daemon rejected profile {value:?}");
                Ok(())
            }
            Self::CpuGovernor => {
                let proxy = crate::dbus_client::daemon_proxy().await?;
                proxy
                    .set_cpu_governor(value)
                    .await
                    .context("daemon set_cpu_governor")?;
                Ok(())
            }
            Self::CpuEpp => {
                let proxy = crate::dbus_client::daemon_proxy().await?;
                proxy
                    .set_cpu_epp(value)
                    .await
                    .context("daemon set_cpu_epp")?;
                Ok(())
            }
            Self::GpuDpmLevel { card } => {
                let proxy = crate::dbus_client::daemon_proxy().await?;
                proxy
                    .set_gpu_dpm_level(card, value)
                    .await
                    .context("daemon set_gpu_dpm_level")?;
                Ok(())
            }
            Self::VCacheMode => {
                let proxy = crate::dbus_client::daemon_proxy().await?;
                proxy
                    .set_vcache_mode(value)
                    .await
                    .context("daemon set_vcache_mode")?;
                Ok(())
            }
        }
    }

    /// Read the knob back and report whether it now holds `expected`.
    ///
    /// This is the step that turns "we sent a request" into "the system
    /// changed", and it is mandatory for every applied change.
    #[must_use]
    pub fn verify(&self, expected: &str) -> Verification {
        match self.read() {
            Some(actual) if actual == expected => Verification::Confirmed,
            Some(actual) => Verification::Mismatch { actual },
            None => Verification::Unreadable,
        }
    }
}

impl fmt::Display for Knob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.title())
    }
}

/// Outcome of reading a knob back after writing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Verification {
    /// The knob now holds the requested value.
    Confirmed,
    /// The write was accepted but the knob holds something else — typically a
    /// second writer (another daemon) contending for the same state.
    Mismatch {
        /// What the knob actually reads now.
        actual: String,
    },
    /// The knob could not be read back, so nothing can be claimed.
    Unreadable,
}

impl Verification {
    /// True only when the change was observed to have taken effect.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

// ── sysfs helpers ────────────────────────────────────────────────────────────

fn cpu_attr_path(cpu: u32, attr: &str) -> PathBuf {
    PathBuf::from(format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/{attr}"))
}

fn read_sysfs(path: &PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn split_sysfs(path: &PathBuf) -> Vec<String> {
    read_sysfs(path)
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_unique() {
        let knobs = [
            Knob::PowerProfile,
            Knob::CpuGovernor,
            Knob::CpuEpp,
            Knob::VCacheMode,
            Knob::GpuDpmLevel {
                card: "card1".into(),
            },
        ];
        let ids: Vec<String> = knobs.iter().map(Knob::id).collect();
        assert_eq!(ids[0], "power_profile");
        assert_eq!(ids[1], "cpu_governor");
        assert_eq!(ids[2], "cpu_epp");
        assert_eq!(ids[3], "vcache_mode");
        // Per-card knobs must not collide with each other.
        assert_eq!(ids[4], "gpu_dpm_level:card1");
        assert_ne!(
            Knob::GpuDpmLevel {
                card: "card0".into()
            }
            .id(),
            Knob::GpuDpmLevel {
                card: "card1".into()
            }
            .id()
        );
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "knob ids must be unique");
    }

    #[test]
    fn journal_round_trip_preserves_knob_identity() {
        // Journals outlive the process; a knob must deserialize to itself.
        let knob = Knob::GpuDpmLevel {
            card: "card1".into(),
        };
        let json = serde_json::to_string(&knob).unwrap();
        assert_eq!(serde_json::from_str::<Knob>(&json).unwrap(), knob);

        let simple = Knob::CpuGovernor;
        let json = serde_json::to_string(&simple).unwrap();
        assert_eq!(serde_json::from_str::<Knob>(&json).unwrap(), simple);
    }

    #[test]
    fn verification_distinguishes_all_three_outcomes() {
        assert!(Verification::Confirmed.is_confirmed());
        assert!(!Verification::Unreadable.is_confirmed());
        assert!(
            !Verification::Mismatch {
                actual: "auto".into()
            }
            .is_confirmed()
        );
    }

    #[test]
    fn verification_serializes_for_the_journal() {
        let v = Verification::Mismatch {
            actual: "balanced".into(),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<Verification>(&json).unwrap(), v);
    }

    #[test]
    fn governor_knob_reflects_this_machine() {
        // Only values the driver lists are accepted (amd-pstate-epp offers just
        // `performance` and `powersave`), so a plan for anything else is
        // rejected before it reaches the helper.
        let allowed = Knob::CpuGovernor.allowed_values();
        if allowed.is_empty() {
            return; // no cpufreq on this host; nothing to assert
        }
        for value in &allowed {
            assert!(Knob::CpuGovernor.accepts(value));
        }
        assert!(!Knob::CpuGovernor.accepts("definitely-not-a-governor"));
    }

    #[test]
    fn absent_hardware_yields_no_allowed_values() {
        let knob = Knob::GpuDpmLevel {
            card: "card999".into(),
        };
        assert!(knob.allowed_values().is_empty());
        assert!(!knob.accepts("high"));
        assert_eq!(knob.read(), None);
    }

    #[tokio::test]
    async fn write_refuses_values_the_machine_rejects() {
        // Must fail on validation, never reach the privileged helper.
        let err = Knob::CpuGovernor
            .write("definitely-not-a-governor")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not accept"));
    }
}
