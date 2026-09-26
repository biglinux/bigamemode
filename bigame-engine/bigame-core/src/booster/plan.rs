//! Planning — deciding what is worth changing on *this* machine.
//!
//! The planner is the only place in the engine allowed to have an opinion, and
//! every change it proposes must answer what it alters, why that can help, what hardware it needs, how support was
//! detected, and how it will be undone. A candidate that cannot answer all five
//! is not planned.
//!
//! It is equally important that the planner is willing to propose **nothing**.
//! A machine already sitting at its best configuration should be told exactly
//! that, not handed a list of no-ops dressed up as optimizations.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::knob::Knob;
use crate::capabilities::Capabilities;
use crate::hardware::{Chassis, Hardware, PowerSource};
use crate::text::{Arg, N_, Text};

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

/// One proposed change, with its full justification.
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

/// Save a sentence of a plan in English, as every earlier build did.
///
/// A plan is kept in the crash-recovery journal, where a record another
/// build cannot read is discarded with the baseline it holds — and Booster's
/// changes could no longer be put back. Nothing shows a journaled plan's
/// sentences, so the journal keeps its format whichever way the application
/// is updated or downgraded.
fn as_english<S: Serializer>(text: &Text, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&text.english())
}

/// Read a sentence of a plan: English, as journaled, or a whole [`Text`].
fn text_or_english<'de, D: Deserializer<'de>>(d: D) -> Result<Text, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Text(Text),
        English(String),
    }
    Ok(match Saved::deserialize(d)? {
        Saved::Text(t) => t,
        Saved::English(s) => Text::raw(s),
    })
}

/// Why a candidate change was not planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Skipped {
    /// The knob does not exist on this hardware.
    Unsupported {
        /// Knob title.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        knob: Text,
        /// What was missing.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        detail: Text,
    },
    /// The knob already holds the value we would have written.
    AlreadyOptimal {
        /// Knob title.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        knob: Text,
        /// The value it already holds.
        value: String,
    },
    /// Applying it here would cost more than it gains.
    NotBeneficial {
        /// Knob title.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        knob: Text,
        /// Why not.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        detail: Text,
    },
    /// Measurement on this machine showed the knob makes things worse.
    ///
    /// Another component is the single writer of this state, so Booster
    /// leaves it alone.
    ///
    /// Two writers of one value is the failure this architecture exists to
    /// prevent: each snapshots and restores independently, so whichever
    /// restores second writes the other's changed value back as a baseline.
    OwnedBy {
        /// Knob title.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        knob: Text,
        /// The component that owns it.
        owner: String,
        /// How it manages the knob.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        detail: Text,
    },
    /// We could not read the current value, so we could not guarantee a
    /// rollback — and an unrestorable change is never worth making.
    NotRestorable {
        /// Knob title.
        #[serde(serialize_with = "as_english", deserialize_with = "text_or_english")]
        knob: Text,
    },
}

/// The title of the scheduler note, which is not a knob of Booster's.
#[must_use]
pub fn scheduler_title() -> Text {
    Text::plain(N_("sched-ext scheduler"))
}

/// What falcond does with the power profile while it holds a game's profile.
fn falcond_is_managing(profile: &str) -> Text {
    Text::with(
        N_(
            "falcond is managing it for '%s'; when the game exits it puts back \
            the profile that was in use when falcond started",
        ),
        [profile],
    )
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

    /// Build the plan for this machine.
    #[must_use]
    pub fn build(hw: &Hardware, caps: &Capabilities, snapshot: &Snapshot) -> Self {
        Self::build_inner(hw, caps, snapshot, &power_profile_owner())
    }

    /// Build a plan with an explicit power-profile owner.
    ///
    /// The owner is a parameter so the arbitration can be tested without a
    /// running falcond.
    #[cfg(test)]
    #[must_use]
    pub fn build_with_owner(
        hw: &Hardware,
        caps: &Capabilities,
        snapshot: &Snapshot,
        owner: &PowerProfileOwner,
    ) -> Self {
        Self::build_inner(hw, caps, snapshot, owner)
    }

    /// The one place a plan is actually assembled.
    fn build_inner(
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
        plan.consider_cpu_governor(hw, caps, snapshot, battery);
        plan.consider_gpu_dpm(hw);
        plan.consider_vcache(hw, caps, snapshot);
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
        // Single writer: where falcond is installed it sets the power profile
        // per game, from that game's profile, and puts one back on exit.
        // Booster writing it too would make two snapshots of one value.
        //
        // Which one it puts back: falcond 2.0.2 records the profile when the
        // service starts, not when a game starts (checked on the reference
        // desktop: started in balanced, switched to power-saver, ran a
        // profiled process — balanced came back). The report says so, because
        // "restores it" reads as "the one before the game".
        if caps.falcond_installed {
            let detail = match owner {
                PowerProfileOwner::Falcond { profile } => falcond_is_managing(profile),
                PowerProfileOwner::Booster => Text::plain(N_(
                    "falcond sets it for each game from the game's profile; when the game \
                     exits it puts back the profile that was in use when falcond started, \
                     not one chosen later",
                )),
            };
            self.skipped.push(Skipped::OwnedBy {
                knob: knob.title_text(),
                owner: "falcond".into(),
                detail,
            });
            return;
        }
        if let PowerProfileOwner::Falcond { profile } = owner {
            self.skipped.push(Skipped::OwnedBy {
                knob: knob.title_text(),
                owner: "falcond".into(),
                detail: falcond_is_managing(profile),
            });
            return;
        }
        if !caps.power_profiles {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title_text(),
                detail: Text::plain(N_("power-profiles-daemon is not reachable")),
            });
            return;
        }
        if !caps
            .power_profiles_available
            .iter()
            .any(|p| p == "performance")
        {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title_text(),
                detail: Text::plain(N_("this platform offers no performance profile")),
            });
            return;
        }
        let Some(from) = snap.value_of(&knob) else {
            self.skipped.push(Skipped::NotRestorable {
                knob: knob.title_text(),
            });
            return;
        };
        if from == "performance" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title_text(),
                value: from.to_owned(),
            });
            return;
        }
        if battery {
            self.skipped.push(Skipped::NotBeneficial {
                knob: knob.title_text(),
                detail: Text::plain(N_(
                    "running on battery — sustained performance mode usually \
                     costs more in throttling than it gains",
                )),
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
    fn consider_cpu_governor(
        &mut self,
        hw: &Hardware,
        caps: &Capabilities,
        snap: &Snapshot,
        battery: bool,
    ) {
        let knob = Knob::CpuGovernor;
        // Where power-profiles-daemon runs, CPU frequency policy is its: on
        // amd-pstate and intel_pstate (active) the power profile sets the
        // energy preference, which the performance governor would pin and
        // lock; on the other drivers distributions map the profile to a
        // governor themselves (BigLinux's power-profiles-daemon-biglinux-cpufreq
        // switches performance/schedutil/conservative on every profile
        // change). Writing it too puts two owners on one setting: on the lab
        // laptop the governor went back to schedutil when falcond restored the
        // balanced profile after a game, while the Turbo report still said
        // performance. It was also measured no faster (Ryzen 7 5700G, SotTR
        // CPU-bound). falcond asks for the performance profile per game.
        if caps.power_profiles {
            self.skipped.push(Skipped::OwnedBy {
                knob: knob.title_text(),
                owner: "power-profiles-daemon".into(),
                detail: if hw.cpu.epp_driven_by_power_profile() {
                    Text::plain(N_(
                        "the power profile sets the CPU's energy preference; forcing the \
                         performance governor would override it, and measured no faster",
                    ))
                } else {
                    Text::plain(N_(
                        "the power profile sets the CPU's frequency policy, and falcond \
                         switches it to performance for each game",
                    ))
                },
            });
            return;
        }
        if hw.cpu.available_governors.is_empty() {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title_text(),
                detail: Text::plain(N_("this machine exposes no CPU frequency control")),
            });
            return;
        }
        if !hw.cpu.supports_governor("performance") {
            let driver = match hw.cpu.scaling_driver.as_deref() {
                Some(name) => Arg::from(name),
                None => Arg::from(Text::plain(N_("this cpufreq driver"))),
            };
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title_text(),
                detail: Text::with(
                    N_("%s accepts only %s"),
                    [
                        driver,
                        Arg::Raw(format!("{:?}", hw.cpu.available_governors)),
                    ],
                ),
            });
            return;
        }
        let Some(from) = snap.value_of(&knob) else {
            self.skipped.push(Skipped::NotRestorable {
                knob: knob.title_text(),
            });
            return;
        };
        if from == "performance" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title_text(),
                value: from.to_owned(),
            });
            return;
        }
        if battery {
            self.skipped.push(Skipped::NotBeneficial {
                knob: knob.title_text(),
                detail: Text::plain(N_("running on battery")),
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

    /// Render GPU `power_dpm_force_performance_level`: never forced.
    ///
    /// On amdgpu, `high` pins the highest *fixed* DPM state and takes the
    /// firmware's boost out of the loop; on a Radeon RX 9060 XT that cost
    /// 7.5 % in a GPU-bound `SuperTuxKart` and 8.3 % in Shadow of the Tomb
    /// Raider — the card held 2.64 GHz and 102 W where `auto` reaches 3.23 GHz
    /// and 162 W. The knob stays in the report so it says why.
    fn consider_gpu_dpm(&mut self, hw: &Hardware) {
        let Some(gpu) = hw.render_gpu() else {
            self.skipped.push(Skipped::Unsupported {
                knob: Text::plain(N_("GPU power level")),
                detail: Text::plain(N_("no render GPU was identified")),
            });
            return;
        };
        let knob = Knob::GpuDpmLevel {
            card: gpu.card.clone(),
        };
        if gpu.dpm_level_path.is_none() {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title_text(),
                detail: Text::with(
                    N_("the %s driver exposes no DPM level control"),
                    [&gpu.driver],
                ),
            });
            return;
        }
        self.skipped.push(Skipped::NotBeneficial {
            knob: knob.title_text(),
            detail: Text::plain(N_(
                "left to the driver: forcing 'high' pins the highest fixed \
                 power state and gives up boost clocks above it, and it has \
                 only ever been measured slower (8.3% in Shadow of the Tomb \
                 Raider on a Radeon RX 9060 XT).",
            )),
        });
    }

    /// AMD 3D V-Cache → `cache`.
    ///
    /// Only meaningful on parts that actually have the stacked cache. Most
    /// gaming workloads prefer the cache die over the higher-clocking one.
    fn consider_vcache(&mut self, hw: &Hardware, caps: &Capabilities, snap: &Snapshot) {
        let knob = Knob::VCacheMode;
        let Some(vcache) = hw.cpu.vcache.as_ref() else {
            self.skipped.push(Skipped::Unsupported {
                knob: knob.title_text(),
                detail: Text::plain(N_("this CPU has no 3D V-Cache")),
            });
            return;
        };
        if caps.falcond_installed {
            self.skipped.push(Skipped::OwnedBy {
                knob: knob.title_text(),
                owner: "falcond".into(),
                detail: Text::plain(N_(
                    "each game's profile sets the V-Cache mode while the game runs",
                )),
            });
            return;
        }
        let from = snap.value_of(&knob).or(vcache.current_mode.as_deref());
        let Some(from) = from else {
            self.skipped.push(Skipped::NotRestorable {
                knob: knob.title_text(),
            });
            return;
        };
        if from == "cache" {
            self.skipped.push(Skipped::AlreadyOptimal {
                knob: knob.title_text(),
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
    /// Two writers contending for `sched_ext` undo each other's changes, so the
    /// Booster never writes it.
    fn note_scheduler(&mut self, caps: &Capabilities) {
        let support = caps.sched_ext.switchable();
        if let Some(reason) = support.describe() {
            self.skipped.push(Skipped::Unsupported {
                knob: scheduler_title(),
                detail: Text::raw(reason),
            });
        } else {
            self.skipped.push(Skipped::NotBeneficial {
                knob: scheduler_title(),
                detail: Text::plain(N_(
                    "falcond owns the scheduler; Booster does not write it to \
                     avoid two controllers contending for the same state",
                )),
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

    /// A machine without falcond: Booster is the only writer of every knob.
    fn caps(performance: bool, ppd: bool) -> Capabilities {
        Capabilities {
            falcond_installed: false,
            falcond_running: false,
            ..caps_with_falcond(performance, ppd)
        }
    }

    /// A machine where falcond is the game backend.
    fn caps_with_falcond(performance: bool, ppd: bool) -> Capabilities {
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
            // A driver where the governor is not the power profile's to set,
            // so Booster owns it. The amd-pstate case has its own tests.
            scaling_driver: Some("acpi-cpufreq".into()),
            available_governors: governors.iter().map(|s| (*s).to_owned()).collect(),
            current_governor: Some("powersave".into()),
            available_epp: Vec::new(),
            current_epp: None,
            amd_pstate_status: None,
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
            pci_slot: "0000:03:00.0".into(),
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
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);

        let ids: Vec<String> = plan.changes.iter().map(|c| c.knob.id()).collect();
        assert!(ids.contains(&"power_profile".to_owned()));
        // The profile it asks for sets the frequency policy: the governor is
        // power-profiles-daemon's, not a second write.
        assert!(!ids.contains(&"cpu_governor".to_owned()));
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::OwnedBy { knob, owner, .. }
            if knob.english().contains("governor") && owner == "power-profiles-daemon"
        )));
        // Without power-profiles-daemon nothing else owns it.
        let bare = Plan::build_with_owner(&h, &caps(true, false), &s, &PowerProfileOwner::Booster);
        assert!(bare.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
        // GPU DPM is left to the driver, and the skip says why.
        assert!(!ids.iter().any(|i| i.starts_with("gpu_dpm")));
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::NotBeneficial { knob, detail } if knob.english().contains("GPU") && detail.english().contains("measured slower")
        )));
        // Every change explains itself.
        assert!(plan.changes.iter().all(|c| !c.rationale.is_empty()));
        assert!(plan.changes.iter().all(|c| c.from != c.to));
    }

    #[test]
    fn plans_nothing_on_a_machine_that_is_already_optimal() {
        // Nothing left to change: performance profile and governor, DPM
        // already `high`.
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
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
        assert!(
            plan.is_empty(),
            "expected no changes, got {:?}",
            plan.changes
        );
        // and it must say why, rather than silently doing nothing
        assert!(plan.skipped.iter().any(|s| matches!(
            s,
            Skipped::AlreadyOptimal { knob, value } if knob.english().contains("Power profile") && value == "performance"
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
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
        assert!(plan.is_empty(), "nothing should be forced on battery");
        assert_eq!(
            plan.skipped
                .iter()
                .filter(|s| matches!(s, Skipped::NotBeneficial { .. }))
                .count(),
            // power profile and GPU DPM; the governor is power-profiles-daemon's
            // (OwnedBy), and the scheduler note is Unsupported here.
            2
        );
    }

    #[test]
    fn never_plans_a_knob_it_cannot_restore() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        // Nothing captured at all.
        let s = snap(&[]);
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
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
        let plan = Plan::build_with_owner(&h, &caps(true, false), &s, &PowerProfileOwner::Booster);
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::Unsupported { knob, .. } if knob.english().contains("governor")
        )));
    }

    #[test]
    fn skips_power_profile_when_the_platform_has_no_performance_mode() {
        let h = hw(PowerSource::Ac, vec![]);
        let s = snap(&[(Knob::PowerProfile, Some("balanced"))]);
        let plan = Plan::build_with_owner(&h, &caps(false, true), &s, &PowerProfileOwner::Booster);
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::PowerProfile));
    }

    #[test]
    fn skips_vcache_on_a_cpu_without_it() {
        let h = hw(PowerSource::Ac, vec![]);
        let plan = Plan::build_with_owner(
            &h,
            &caps(true, true),
            &snap(&[]),
            &PowerProfileOwner::Booster,
        );
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::Unsupported { knob, detail } if knob.english().contains("V-Cache") && detail.english().contains("no 3D V-Cache")
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
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
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
            loader_installed: true,
            loader_service: true,
        };
        let plan = Plan::build_with_owner(&h, &c, &s, &PowerProfileOwner::Booster);
        // Even when fully switchable, the scheduler stays falcond's.
        assert!(
            plan.changes
                .iter()
                .all(|ch| ch.knob != Knob::VCacheMode || ch.knob == Knob::VCacheMode)
        );
        assert!(plan.skipped.iter().any(|s| matches!(
            s, Skipped::NotBeneficial { knob, detail } if knob.english().contains("sched-ext") && detail.english().contains("falcond owns")
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
            sk, Skipped::OwnedBy { knob, owner, detail }
            if knob.english().contains("Power profile") && owner == "falcond" && detail.english().contains("Cyberpunk")
        )));
        // Nor with power-profiles-daemon for the governor that profile sets.
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
    }

    #[test]
    fn the_governor_is_left_to_power_profiles_daemon_on_every_driver() {
        // The lab laptop: intel_cpufreq (passive), where BigLinux maps the
        // power profile to a governor on every profile change.
        let mut h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        h.cpu.scaling_driver = Some("intel_cpufreq".into());
        let s = snap(&[(Knob::CpuGovernor, Some("schedutil"))]);
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
        assert!(plan.skipped.iter().any(|sk| matches!(
            sk, Skipped::OwnedBy { owner, detail, .. }
            if owner == "power-profiles-daemon" && detail.english().contains("falcond")
        )));
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
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
        let json = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.changes.len(), plan.changes.len());
        let english = |p: &Plan| serde_json::to_value(&p.skipped).unwrap();
        assert_eq!(english(&back), english(&plan));
    }

    #[test]
    fn the_journal_keeps_its_sentences_in_english() {
        // A build from before translation must still read a journal this one
        // wrote, or a downgrade would discard the baseline.
        let plan = Plan {
            changes: Vec::new(),
            skipped: vec![Skipped::OwnedBy {
                knob: Knob::GpuDpmLevel {
                    card: "card1".into(),
                }
                .title_text(),
                owner: "falcond".into(),
                detail: falcond_is_managing("Proton"),
            }],
        };
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["skipped"][0]["knob"], "GPU power level (card1)");
        assert!(
            json["skipped"][0]["detail"]
                .as_str()
                .unwrap()
                .starts_with("falcond is managing it for 'Proton'")
        );
    }

    #[test]
    fn a_plan_journaled_with_english_sentences_still_loads() {
        // Every build journals titles and details as plain strings; failing
        // to read them would discard the baseline.
        let old = r#"{"changes":[],"skipped":[
            {"reason":"owned_by","knob":"Power profile","owner":"falcond","detail":"per game"},
            {"reason":"not_restorable","knob":"CPU governor"}]}"#;
        let plan: Plan = serde_json::from_str(old).unwrap();
        assert!(matches!(
            &plan.skipped[0],
            Skipped::OwnedBy { knob, detail, .. }
            if knob.english() == "Power profile" && detail.english() == "per game"
        ));
        assert!(matches!(
            &plan.skipped[1],
            Skipped::NotRestorable { knob } if knob.english() == "CPU governor"
        ));
    }

    fn amd_pstate(mut h: Hardware) -> Hardware {
        h.cpu.scaling_driver = Some("amd-pstate-epp".into());
        h.cpu.amd_pstate_status = Some("active".into());
        h
    }

    #[test]
    fn with_falcond_installed_booster_leaves_it_the_power_profile() {
        let h = hw(PowerSource::Ac, vec![dgpu("card1", true)]);
        let s = snap(&[(Knob::PowerProfile, Some("balanced"))]);
        let plan = Plan::build_with_owner(
            &h,
            &caps_with_falcond(true, true),
            &s,
            &PowerProfileOwner::Booster,
        );
        // Even with no game running: falcond is the single writer, per game.
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::PowerProfile));
        assert!(plan.skipped.iter().any(|sk| matches!(
            sk, Skipped::OwnedBy { knob, owner, .. }
            if knob.english().contains("Power profile") && owner == "falcond"
        )));
    }

    #[test]
    fn on_amd_pstate_the_governor_belongs_to_the_power_profile() {
        let h = amd_pstate(hw(PowerSource::Ac, vec![dgpu("card1", true)]));
        let s = snap(&[(Knob::CpuGovernor, Some("powersave"))]);
        let plan = Plan::build_with_owner(&h, &caps(true, true), &s, &PowerProfileOwner::Booster);
        assert!(!plan.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
        assert!(plan.skipped.iter().any(|sk| matches!(
            sk, Skipped::OwnedBy { knob, owner, .. }
            if knob.english().contains("governor") && owner == "power-profiles-daemon"
        )));

        // Without power-profiles-daemon nothing else sets it, so Booster may.
        let alone = Plan::build_with_owner(&h, &caps(true, false), &s, &PowerProfileOwner::Booster);
        assert!(alone.changes.iter().any(|c| c.knob == Knob::CpuGovernor));
    }

    #[test]
    fn an_already_tuned_amd_pstate_machine_gets_an_empty_plan_with_reasons() {
        // Ryzen 7 5700G on amd-pstate-epp, falcond and power-profiles-daemon
        // present, resting at performance/auto: every knob has an owner or a
        // reason, and nothing is written.
        let h = amd_pstate(hw(PowerSource::Ac, vec![dgpu("card1", true)]));
        let s = snap(&[
            (Knob::PowerProfile, Some("performance")),
            (Knob::CpuGovernor, Some("performance")),
            (
                Knob::GpuDpmLevel {
                    card: "card1".into(),
                },
                Some("auto"),
            ),
        ]);
        let plan = Plan::build_with_owner(
            &h,
            &caps_with_falcond(true, true),
            &s,
            &PowerProfileOwner::Booster,
        );
        assert!(plan.is_empty(), "{:?}", plan.changes);
        let owned = plan
            .skipped
            .iter()
            .filter(|sk| matches!(sk, Skipped::OwnedBy { .. }))
            .count();
        assert_eq!(
            owned, 2,
            "power profile to falcond, governor to the profile"
        );
    }
}
