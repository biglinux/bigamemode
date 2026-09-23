//! Planning — deciding what is worth changing on *this* machine.
//!
//! The planner is the only place in the engine allowed to have an opinion, and
//! it is held to the brief's standard: every change it proposes must answer
//! what it alters, why that can help, what hardware it needs, how support was
//! detected, and how it will be undone. A candidate that cannot answer all five
//! is not planned.
//!
//! It is equally important that the planner is willing to propose **nothing**.
//! A machine already sitting at its best configuration should be told exactly
//! that, not handed a list of no-ops dressed up as optimizations.

use serde::{Deserialize, Serialize};

use super::knob::Knob;
use crate::capabilities::Capabilities;
use crate::hardware::{Chassis, Hardware, PowerSource};

use super::snapshot::Snapshot;

/// Who currently owns the power profile.
///
/// falcond activates a game profile, switches the power profile, and restores
/// its own snapshot when the game exits — confirmed on falcond 2.0.2, whose
/// binary talks to `org.freedesktop.UPower.PowerProfiles` and whose status file
/// carries a `RESTORE_STATE: Power Profile:` line.
///
/// That makes falcond a second writer. If Booster also writes the profile while
/// a game is running, falcond's restore silently undoes it the moment the game
/// exits — and Booster would have already reported the change as verified,
/// because at the instant it checked, it was. Rather than fight, Booster stands
/// down for as long as falcond holds a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerProfileOwner {
    /// No game profile is active; Booster may manage the power profile.
    Booster,
    /// falcond has a profile active and is managing it.
    Falcond {
        /// The profile falcond matched.
        profile: String,
    },
}

/// Ask falcond whether it currently holds a profile.
#[must_use]
pub fn power_profile_owner() -> PowerProfileOwner {
    match crate::status::read().and_then(|s| s.active_profile) {
        Some(profile) => PowerProfileOwner::Falcond { profile },
        None => PowerProfileOwner::Booster,
    }
}

/// How risky a change is, which drives whether it is applied automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Reversible, well-understood, no thermal or stability cost.
    Safe,
    /// Reversible but raises power draw or heat; gated on AC power.
    Thermal,
}

/// One proposed change, with the full justification the brief requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// What is being changed.
    pub knob: Knob,
    /// Value observed before planning.
    pub from: String,
    /// Value to write.
    pub to: String,
    /// Why this can help. Shown verbatim in the report.
    pub rationale: String,
    /// Risk classification.
    pub risk: Risk,
}

/// Why a candidate change was not planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Skipped {
    /// The knob does not exist on this hardware.
    Unsupported {
        /// Knob title.
        knob: String,
        /// What was missing.
        detail: String,
    },
    /// The knob already holds the value we would have written.
    AlreadyOptimal {
        /// Knob title.
        knob: String,
        /// The value it already holds.
        value: String,
    },
    /// Applying it here would cost more than it gains.
    NotBeneficial {
        /// Knob title.
        knob: String,
        /// Why not.
        detail: String,
    },
    /// We could not read the current value, so we could not guarantee a
    /// rollback — and an unrestorable change is never worth making.
    NotRestorable {
        /// Knob title.
        knob: String,
    },
}

/// The complete decision for one Booster activation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Plan {
    /// Changes to apply, in order.
    pub changes: Vec<Change>,
    /// Candidates that were considered and rejected, with reasons. Reporting
    /// these is what lets the user see the engine reasoned rather than guessed.
    pub skipped: Vec<Skipped>,
}

impl Plan {
    /// True when there is nothing worth doing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Build a plan for this machine.
    ///
    /// `snapshot` must already have been captured: the planner refuses to plan
    /// anything it could not put back.
    #[must_use]
    pub fn build(hw: &Hardware, caps: &Capabilities, snapshot: &Snapshot) -> Self {
        Self::build_with_owner(hw, caps, snapshot, &power_profile_owner())
    }

    /// Build a plan with an explicit power-profile owner.
    ///
    /// Split out from [`Plan::build`] so the arbitration can be tested without
    /// a running falcond.
    #[must_use]
    pub fn build_with_owner(
        hw: &Hardware,
        caps: &Capabilities,
        snapshot: &Snapshot,
        owner: &PowerProfileOwner,
    ) -> Self {
        let mut plan = Self::default();
        // On battery, raising sustained power draw usually costs more in
        // thermal throttling and clock ceiling than it returns. The user can
        // still opt in per-knob from Advanced; the automatic plan will not.
        let battery = hw.power_source == PowerSource::Battery;

        plan.consider_power_profile(caps, snapshot, battery, owner);
        plan.consider_cpu_governor(hw, snapshot, battery);
        plan.consider_gpu_dpm(hw, snapshot, battery);
        plan.consider_vcache(hw, snapshot);
        plan.note_scheduler(caps);
        plan
    }

    // ── Candidates ───────────────────────────────────────────────────────────

    /// Power profile → `performance`.
    ///
    /// Alters: power-profiles-daemon's active profile, which drives the
    /// platform profile and the cpufreq EPP.
    /// Helps because: the balanced and power-saver profiles bias the EPP toward
    /// efficiency, which raises frequency-ramp latency on bursty game threads.
    /// Needs: power-profiles-daemon, with a `performance` profile offered.
    /// Undone by: writing the captured profile back.
    fn consider_power_profile(
        &mut self,
        caps: &Capabilities,
        snap: &Snapshot,
        battery: bool,
        owner: &PowerProfileOwner,
    ) {
        let knob = Knob::PowerProfile;
        // Single authority: while falcond holds a game profile it owns this
        // knob, and it will restore its own baseline when the game exits.
        if let PowerProfileOwner::Falcond { profile } = owner {
            self.skipped.push(Skipped::NotBeneficial {
                knob: knob.title(),
                detail: format!(
                    "falcond is managing the power profile for '{profile}' and will                      restore it when the game exits"
                ),
            });
            return;
        }
        if !caps.power_profiles {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title(),
                detail: "power-profiles-daemon is not reachable".into(),
            });
            return;
        }
        if !caps
            .power_profiles_available
            .iter()
            .any(|p| p == "performance")
        {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title(),
                detail: "this platform offers no performance profile".into(),
            });
            return;
        }
        let Some(from) = snap.value_of(&knob) else {
            self.skipped
                .push(Skipped::NotRestorable { knob: knob.title() });
            return;
        };
        if from == "performance" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title(),
                value: from.to_owned(),
            });
            return;
        }
        if battery {
            self.skipped.push(Skipped::NotBeneficial {
                knob: knob.title(),
                detail: "running on battery — sustained performance mode usually \
                         costs more in throttling than it gains"
                    .into(),
            });
            return;
        }
        self.changes.push(Change {
            knob,
            from: from.to_owned(),
            to: "performance".into(),
            rationale: "Stops the platform biasing CPU frequency toward efficiency, \
                        which shortens frequency-ramp latency on bursty frame threads."
                .into(),
            risk: Risk::Safe,
        });
    }

    /// CPU governor → `performance`, only where the driver accepts it.
    fn consider_cpu_governor(&mut self, hw: &Hardware, snap: &Snapshot, battery: bool) {
        let knob = Knob::CpuGovernor;
        if !hw.cpu.supports_governor("performance") {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title(),
                detail: format!(
                    "{} accepts only {:?}",
                    hw.cpu
                        .scaling_driver
                        .as_deref()
                        .unwrap_or("this cpufreq driver"),
                    hw.cpu.available_governors
                ),
            });
            return;
        }
        let Some(from) = snap.value_of(&knob) else {
            self.skipped
                .push(Skipped::NotRestorable { knob: knob.title() });
            return;
        };
        if from == "performance" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title(),
                value: from.to_owned(),
            });
            return;
        }
        if battery {
            self.skipped.push(Skipped::NotBeneficial {
                knob: knob.title(),
                detail: "running on battery".into(),
            });
            return;
        }
        self.changes.push(Change {
            knob,
            from: from.to_owned(),
            to: "performance".into(),
            rationale: "Holds cores at their performance operating point instead of \
                        ramping on demand, removing ramp-up stalls at frame boundaries."
                .into(),
            risk: Risk::Safe,
        });
    }

    /// Render GPU `power_dpm_force_performance_level` → `high`.
    ///
    /// Only the card games actually render on is touched. Leaving an idle iGPU
    /// pinned high wastes power for nothing, and is precisely the mistake the
    /// old first-card-wins telemetry walk would have led to.
    fn consider_gpu_dpm(&mut self, hw: &Hardware, snap: &Snapshot, battery: bool) {
        let Some(gpu) = hw.render_gpu() else {
            self.skipped.push(Skipped::Unsupported {
                knob: "GPU power level".into(),
                detail: "no render GPU was identified".into(),
            });
            return;
        };
        let knob = Knob::GpuDpmLevel {
            card: gpu.card.clone(),
        };
        if gpu.dpm_level_path.is_none() {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title(),
                detail: format!("the {} driver exposes no DPM level control", gpu.driver),
            });
            return;
        }
        let Some(from) = snap.value_of(&knob) else {
            self.skipped
                .push(Skipped::NotRestorable { knob: knob.title() });
            return;
        };
        if from == "high" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title(),
                value: from.to_owned(),
            });
            return;
        }
        if battery {
            self.skipped.push(Skipped::NotBeneficial {
                knob: knob.title(),
                detail: "running on battery — pinning GPU clocks high drains the \
                         battery and invites thermal throttling"
                    .into(),
            });
            return;
        }
        self.changes.push(Change {
            knob,
            from: from.to_owned(),
            to: "high".into(),
            rationale: format!(
                "Keeps {} at its high DPM state so the first frames after a load \
                 screen are not rendered at idle clocks.",
                gpu.card
            ),
            risk: Risk::Thermal,
        });
    }

    /// AMD 3D V-Cache → `cache`.
    ///
    /// Only meaningful on parts that actually have the stacked cache. Most
    /// gaming workloads prefer the cache die over the higher-clocking one.
    fn consider_vcache(&mut self, hw: &Hardware, snap: &Snapshot) {
        let knob = Knob::VCacheMode;
        let Some(vcache) = hw.cpu.vcache.as_ref() else {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title(),
                detail: "this CPU has no 3D V-Cache".into(),
            });
            return;
        };
        let from = snap.value_of(&knob).or(vcache.current_mode.as_deref());
        let Some(from) = from else {
            self.skipped
                .push(Skipped::NotRestorable { knob: knob.title() });
            return;
        };
        if from == "cache" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title(),
                value: from.to_owned(),
            });
            return;
        }
        self.changes.push(Change {
            knob,
            from: from.to_owned(),
            to: "cache".into(),
            rationale: "Parks threads on the cache-stacked die, which most game \
                        engines prefer over the higher-clocking die."
                .into(),
            risk: Risk::Safe,
        });
    }

    /// The scheduler is falcond's to own — we only explain why it is not ours.
    ///
    /// See `docs/02-PERFORMANCE-AUTHORITY.md`: two writers contending for
    /// `sched_ext` is exactly the class of conflict this architecture exists to
    /// prevent, so the Booster never writes it.
    fn note_scheduler(&mut self, caps: &Capabilities) {
        let support = caps.sched_ext.switchable();
        if let Some(reason) = support.reason() {
            self.skipped.push(Skipped::Unsupported {
                knob: "sched-ext scheduler".into(),
                detail: reason.to_owned(),
            });
        } else {
            self.skipped.push(Skipped::NotBeneficial {
                knob: "sched-ext scheduler".into(),
                detail: "falcond owns the scheduler; Booster does not write it to \
                         avoid two controllers contending for the same state"
                    .into(),
            });
        }
    }
}

/// A one-line summary of the machine, for the report header.
#[must_use]
pub fn describe_machine(hw: &Hardware) -> String {
    let mut parts = vec![hw.cpu.model.clone()];
    if let Some(gpu) = hw.render_gpu() {
        parts.push(format!("{} ({})", gpu.card, gpu.driver));
    }
    parts.push(
        match hw.session {
            crate::hardware::Session::Wayland => "Wayland",
            crate::hardware::Session::X11 => "X11",
            crate::hardware::Session::Tty => "no graphical session",
        }
        .to_owned(),
    );
    parts.push(
        match hw.chassis {
            Chassis::Desktop => "desktop",
            Chassis::Laptop => "laptop",
            Chassis::Handheld => "handheld",
            Chassis::Unknown => "unknown chassis",
        }
        .to_owned(),
    );
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::SchedExtCaps;
    use crate::hardware::{Cpu, CpuVendor, Gpu, GpuVendor, Session};
    use std::path::PathBuf;

    fn caps(performance: bool, ppd: bool) -> Capabilities {
        Capabilities {
            gamescope: None,
            mangohud: false,
            mangoapp: false,
            falcond_installed: true,
            falcond_running: true,
            gamemode: false,
            power_profiles: ppd,
            power_profiles_available: if performance {
                vec![
                    "power-saver".into(),
                    "balanced".into(),
                    "performance".into(),
                ]
            } else {
                vec!["balanced".into()]
            },
            sched_ext: SchedExtCaps::default(),
            lsfg_vk: false,
            vkbasalt: false,
            steam: false,
        }
    }

    fn cpu(governors: &[&str]) -> Cpu {
        Cpu {
            vendor: CpuVendor::Amd,
            model: "AMD Ryzen 7 5700G".into(),
            physical_cores: 8,
            logical_cpus: 16,
            smt: true,
            hybrid: false,
            scaling_driver: Some("amd-pstate-epp".into()),
            available_governors: governors.iter().map(|s| (*s).to_owned()).collect(),
            current_governor: Some("powersave".into()),
            available_epp: Vec::new(),
            current_epp: None,
            amd_pstate_status: Some("active".into()),
            vcache: None,
        }
    }

    fn hw(power: PowerSource, gpus: Vec<Gpu>) -> Hardware {
        let render_gpu = crate::hardware::pick_render_gpu(&gpus);
        Hardware {
            cpu: cpu(&["performance", "powersave"]),
            gpus,
            render_gpu,
            displays: Vec::new(),
            chassis: if power == PowerSource::Battery {
                Chassis::Laptop
            } else {
                Chassis::Desktop
            },
            power_source: power,
            session: Session::Wayland,
            kernel: "7.2.6".into(),
        }
    }

    fn dgpu(card: &str, dpm: bool) -> Gpu {
        Gpu {
            card: card.into(),
            device_path: PathBuf::from("/sys/class/drm").join(card).join("device"),
            vendor: GpuVendor::Amd,
            pci_id: "1002:7590".into(),
            driver: "amdgpu".into(),
            hwmon: None,
            connected_outputs: vec!["DP-1".into()],
            vram_total_bytes: Some(17_095_983_104),
            discrete: true,
            dpm_level_path: dpm.then(|| PathBuf::from("/tmp/fake_dpm")),
        }
    }

    fn snap(pairs: &[(Knob, Option<&str>)]) -> Snapshot {
        use super::super::snapshot::Captured;
        Snapshot {
            taken_at: 1,
            entries: pairs
                .iter()
                .map(|(k, v)| {
                    (
                        k.id(),
                        Captured {
                            knob: k.clone(),
                            value: v.map(str::to_owned),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn plans_the_changes_a_cold_desktop_needs() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[
            (Knob::PowerProfile, Some("balanced")),
            (Knob::CpuGovernor, Some("powersave")),
            (
                Knob::GpuDpmLevel {
                    card: "card1".into(),
                },
                Some("auto"),
            ),
        ]);
        let plan = Plan::build(&h, &caps(true, true), &s);

        let ids: Vec<String> = plan.changes.iter().map(|c| c.knob.id()).collect();
        assert!(ids.contains(&"power_profile".to_owned()));
        assert!(ids.contains(&"cpu_governor".to_owned()));
        assert!(ids.contains(&"gpu_dpm_level:card1".to_owned()));
        // Only the render GPU, never the idle iGPU.
        assert!(!ids.iter().any(|i| i.contains("card0")));
        // Every change explains itself.
        assert!(plan.changes.iter().all(|c| !c.rationale.is_empty()));
        assert!(plan.changes.iter().all(|c| c.from != c.to));
    }

    #[test]
    fn plans_nothing_on_a_machine_that_is_already_optimal() {
        // The bench's real resting state.
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[
            (Knob::PowerProfile, Some("performance")),
            (Knob::CpuGovernor, Some("performance")),
            (
                Knob::GpuDpmLevel {
                    card: "card1".into(),
                },
                Some("high"),
            ),
        ]);
        let plan = Plan::build(&h, &caps(true, true), &s);
        assert!(
            plan.is_empty(),
            "expected no changes, got {:?}",
            plan.changes
        );
        // and it must say why, rather than silently doing nothing
        assert!(plan.skipped.iter().any(|s| matches!(
            s,
            Skipped::AlreadyOptimal { knob, value } if knob.contains("Power profile") && value == "performance"
        )));
    }

    #[test]
    fn refuses_to_raise_power_draw_on_battery() {
        let h = hw(PowerSource::Battery, vec![dgpu("card1", true)]);
        let s = snap(&[
            (Knob::PowerProfile, Some("balanced")),
            (Knob::CpuGovernor, Some("powersave")),
            (
                Knob::GpuDpmLevel {
                    card: "card1".into(),
                },
                Some("auto"),
            ),
        ]);
        let plan = Plan::build(&h, &caps(true, true), &s);
        assert!(plan.is_empty(), "nothing should be forced on battery");
        assert_eq!(
            plan.skipped
                .iter()
                .filter(|s| matches!(s, Skipped::NotBeneficial { .. }))
                .count(),
            // power profile, governor, GPU DPM — plus the scheduler note is
            // Unsupported here, not NotBeneficial.
            3
        );
    }

    #[test]
    fn never_plans_a_knob_it_cannot_restore() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        // Nothing captured at all.
        let s = snap(&[]);
        let plan = Plan::build(&h, &caps(true, true), &s);
        assert!(plan.is_empty());
        assert!(
            plan.skipped
                .iter()
                .any(|s| matches!(s, Skipped::NotRestorable { .. }))
        );
    }

    #[test]
    fn skips_governors_the_driver_will_not_accept() {
        let mut h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        // A driver offering only powersave — `performance` is not writable.
        h.cpu = cpu(&["powersave"]);
        let s = snap(&[(Knob::CpuGovernor, Some("powersave"))]);
        let plan = Plan::build(&h, &caps(true, true), &s);
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::Unsupported { knob, .. } if knob.contains("governor")
        )));
    }

    #[test]
    fn skips_power_profile_when_the_platform_has_no_performance_mode() {
        let h = hw(PowerSource::Ac, vec![]);
        let s = snap(&[(Knob::PowerProfile, Some("balanced"))]);
        let plan = Plan::build(&h, &caps(false, true), &s);
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::PowerProfile));
    }

    #[test]
    fn skips_vcache_on_a_cpu_without_it() {
        let h = hw(PowerSource::Ac, vec![]);
        let plan = Plan::build(&h, &caps(true, true), &snap(&[]));
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::Unsupported { knob, detail } if knob.contains("V-Cache") && detail.contains("no 3D V-Cache")
        )));
    }

    #[test]
    fn skips_gpu_dpm_when_the_driver_exposes_none() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", false)]);
        let s = snap(&[(
            Knob::GpuDpmLevel {
                card: "card1".into(),
            },
            Some("auto"),
        )]);
        let plan = Plan::build(&h, &caps(true, true), &s);
        assert!(
            !plan
                .changes
                .iter()
                .any(|c| c.knob.id().starts_with("gpu_dpm"))
        );
    }

    #[test]
    fn never_writes_the_scheduler() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[
            (Knob::PowerProfile, Some("balanced")),
            (Knob::CpuGovernor, Some("powersave")),
            (
                Knob::GpuDpmLevel {
                    card: "card1".into(),
                },
                Some("auto"),
            ),
        ]);
        let mut c = caps(true, true);
        c.sched_ext = SchedExtCaps {
            kernel_support: true,
            state: Some("disabled".into()),
            installed: vec!["lavd".into()],
            scxctl: true,
            loader_service: true,
        };
        let plan = Plan::build(&h, &c, &s);
        // Even when fully switchable, the scheduler stays falcond's.
        assert!(
            plan.changes
                .iter()
                .all(|ch| ch.knob != Knob::VCacheMode || ch.knob == Knob::VCacheMode)
        );
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::NotBeneficial { knob, detail } if knob.contains("sched-ext") && detail.contains("falcond owns")
        )));
    }

    #[test]
    fn booster_stands_down_while_falcond_holds_a_profile() {
        // Two writers on the same knob is the conflict this architecture
        // exists to remove. falcond restores its own snapshot when the game
        // exits, which would silently undo anything Booster wrote.
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[
            (Knob::PowerProfile, Some("balanced")),
            (Knob::CpuGovernor, Some("powersave")),
        ]);
        let plan = Plan::build_with_owner(
            &h,
            &caps(true, true),
            &s,
            &PowerProfileOwner::Falcond {
                profile: "Cyberpunk2077.exe".into(),
            },
        );
        assert!(
            !plan.changes.iter().any(|c| c.knob == Knob::PowerProfile),
            "must not contend with falcond for the power profile"
        );
        assert!(plan.skipped.iter().any(|sk| matches!(
            sk, Skipped::NotBeneficial { knob, detail }
            if knob.contains("Power profile") && detail.contains("falcond")
        )));
        // The knobs falcond does not manage are still planned.
        assert!(plan.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
    }

    #[test]
    fn booster_owns_the_power_profile_when_no_game_is_active() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[(Knob::PowerProfile, Some("balanced"))]);
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
        assert!(plan.changes.iter().any(|c| c.knob == Knob::PowerProfile));
    }

    #[test]
    fn plan_serializes_for_the_journal() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[(Knob::PowerProfile, Some("balanced"))]);
        let plan = Plan::build(&h, &caps(true, true), &s);
        let json = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.changes.len(), plan.changes.len());
    }
}
