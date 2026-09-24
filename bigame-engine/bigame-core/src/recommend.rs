//! A recommended falcond profile for a game this machine has not seen before.
//!
//! Every value carries where it came from ([`Evidence`]) and why. Nothing is
//! chosen because of its name: a scheduler is not picked because the game is
//! a shooter, and "performance" is not assumed to be faster because of what it
//! is called — the one GPU setting named for speed measured 8 % slower here.
//!
//! Sources, in the order they are consulted:
//!
//! 1. what the hardware has (V-Cache, battery),
//! 2. what the kernel and tools offer (sched-ext, `scx_loader`),
//! 3. what falcond's own base profiles do,
//! 4. what was measured on this machine.
//!
//! The profile contains **only falcond's fields**. Earlier profiles written by
//! this project also carried BiGame-mode's frame-generation settings into
//! falcond's directory, where falcond ignores them and nothing reads them back.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::capabilities::Capabilities;
use crate::hardware::{Hardware, PowerSource};
use crate::running::GameIdentity;

/// How much a decision rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// A fact about the game or machine, not a performance claim.
    Fact,
    /// Chosen because the hardware or software supports it; not measured.
    CapabilityOnly,
    /// What falcond's own profiles do; not measured here.
    UpstreamDefault,
    /// Measured on this machine, and the difference was real.
    LocallyMeasured,
    /// Measured on this machine and found slower, so avoided.
    Regression,
    /// Not possible on this machine.
    Unsupported,
}

impl Evidence {
    /// A short label for the UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fact => "Fact",
            Self::CapabilityOnly => "Supported, not measured",
            Self::UpstreamDefault => "falcond default, not measured here",
            Self::LocallyMeasured => "Measured on this machine",
            Self::Regression => "Measured slower — avoided",
            Self::Unsupported => "Not available on this machine",
        }
    }
}

/// One value in the profile, and why it has that value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// The falcond key.
    pub key: String,
    /// The value written.
    pub value: String,
    /// What it rests on.
    pub evidence: Evidence,
    /// Why, in a sentence.
    pub why: String,
}

/// A profile ready to save, with its reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recommendation {
    /// The profile's name, which is the process falcond matches.
    pub name: String,
    /// Each value, in file order.
    pub decisions: Vec<Decision>,
}

impl Recommendation {
    /// The profile in falcond's on-disk format.
    ///
    /// Strings are quoted and enums and booleans are bare, which is what
    /// falcond's `otter_conf` parser expects.
    #[must_use]
    pub fn to_falcond(&self) -> String {
        let mut out = format!("name = \"{}\"\n", self.name.replace('"', ""));
        for d in &self.decisions {
            if d.key == "name" {
                continue;
            }
            let _ = writeln!(out, "{} = {}", d.key, d.value);
        }
        out
    }
}

fn decide(key: &str, value: &str, evidence: Evidence, why: impl Into<String>) -> Decision {
    Decision {
        key: key.to_owned(),
        value: value.to_owned(),
        evidence,
        why: why.into(),
    }
}

/// Recommend a profile for `game` on this machine.
#[must_use]
pub fn recommend(game: &GameIdentity, hardware: &Hardware, caps: &Capabilities) -> Recommendation {
    let mut decisions = vec![decide(
        "name",
        &game.process_name,
        Evidence::Fact,
        "the process falcond sees for this game; the profile applies whenever it runs",
    )];

    let battery = hardware.power_source == PowerSource::Battery;
    decisions.push(if battery {
        decide(
            "performance_mode",
            "false",
            Evidence::CapabilityOnly,
            "on battery, holding the performance power profile costs more in heat and \
             throttling than it returns",
        )
    } else {
        decide(
            "performance_mode",
            "true",
            Evidence::UpstreamDefault,
            "switches to the performance power profile while the game runs and back \
             afterwards, as falcond's own profiles do; on the reference machine it \
             measured no faster than balanced, so it is not a speed claim",
        )
    });

    let scheduler = match caps.sched_ext.switchable().describe() {
        None => decide(
            "scx_sched",
            "none",
            Evidence::CapabilityOnly,
            "sched-ext is available, but no scheduler has been measured faster for this \
             game here; calibrating the game can change that",
        ),
        Some(why) => decide("scx_sched", "none", Evidence::Unsupported, why),
    };
    decisions.push(scheduler);
    decisions.push(decide(
        "scx_sched_props",
        "default",
        Evidence::Fact,
        "no scheduler is set, so its mode has no effect",
    ));

    decisions.push(if hardware.cpu.vcache.is_some() {
        decide(
            "vcache_mode",
            "cache",
            Evidence::UpstreamDefault,
            "prefers the cache-stacked CCD while the game runs, as falcond's profiles do",
        )
    } else {
        decide(
            "vcache_mode",
            "none",
            Evidence::Unsupported,
            "this CPU has no 3D V-Cache",
        )
    });

    decisions.push(decide(
        "idle_inhibit",
        "true",
        Evidence::CapabilityOnly,
        "keeps the screen from blanking while playing with a controller",
    ));

    Recommendation {
        name: game.process_name.clone(),
        decisions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::running::{Graphics, Runtime};

    fn game(name: &str) -> GameIdentity {
        GameIdentity {
            display_name: "Shadow of the Tomb Raider".into(),
            steam_app_id: Some("750920".into()),
            install_path: None,
            compatdata_path: None,
            pid: 1,
            process_name: name.into(),
            executable: format!("S:\\x\\{name}"),
            runtime: Runtime::Proton("Proton - Experimental".into()),
            graphics: Graphics::Vkd3dProton,
            render_card: Some("card1".into()),
            tree: vec![],
        }
    }

    fn machine(battery: bool, vcache: bool) -> (Hardware, Capabilities) {
        let mut hw = Hardware::detect();
        hw.power_source = if battery {
            PowerSource::Battery
        } else {
            PowerSource::Ac
        };
        hw.cpu.vcache = vcache.then(|| crate::hardware::VCacheDevice {
            mode_path: "/sys/x".into(),
            current_mode: Some("frequency".into()),
        });
        (hw, Capabilities::detect())
    }

    #[test]
    fn the_profile_is_keyed_on_the_process_and_holds_only_falcond_fields() {
        let (hw, caps) = machine(false, false);
        let text = recommend(&game("SOTTR.exe"), &hw, &caps).to_falcond();
        assert!(text.starts_with("name = \"SOTTR.exe\"\n"), "{text}");
        for key in text
            .lines()
            .filter_map(|l| l.split_once(" = ").map(|(k, _)| k))
        {
            assert!(
                [
                    "name",
                    "performance_mode",
                    "scx_sched",
                    "scx_sched_props",
                    "vcache_mode",
                    "idle_inhibit"
                ]
                .contains(&key),
                "{key} is not a falcond field"
            );
        }
        assert!(!text.contains("script"), "script hooks are never written");
    }

    #[test]
    fn nothing_is_claimed_that_was_not_measured() {
        let (hw, caps) = machine(false, false);
        let rec = recommend(&game("SOTTR.exe"), &hw, &caps);
        assert!(
            rec.decisions
                .iter()
                .all(|d| d.evidence != Evidence::LocallyMeasured),
            "no calibration was supplied, so nothing may say it was measured"
        );
        let perf = rec
            .decisions
            .iter()
            .find(|d| d.key == "performance_mode")
            .unwrap();
        assert!(perf.why.contains("not a speed claim"), "{}", perf.why);
    }

    #[test]
    fn hardware_decides_vcache_and_battery() {
        let (hw, caps) = machine(true, true);
        let rec = recommend(&game("Game.exe"), &hw, &caps);
        let get = |k: &str| {
            rec.decisions
                .iter()
                .find(|d| d.key == k)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(get("performance_mode"), "false");
        assert_eq!(get("vcache_mode"), "cache");

        let (hw, caps) = machine(false, false);
        let rec = recommend(&game("Game.exe"), &hw, &caps);
        let vcache = rec
            .decisions
            .iter()
            .find(|d| d.key == "vcache_mode")
            .unwrap();
        assert_eq!(vcache.value, "none");
        assert_eq!(vcache.evidence, Evidence::Unsupported);
    }
}
