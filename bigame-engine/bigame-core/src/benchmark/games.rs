//! Benchmark providers for installed games.
//!
//! Several commercial titles ship an excellent built-in benchmark — a fixed
//! camera path over a fixed scene, which is a far better measure of gaming
//! performance than any synthetic loop. What varies is how much of that can be
//! driven without a person in the chair, and this module's job is to say which,
//! per title, truthfully.
//!
//! The four states matter because they lead somewhere different:
//!
//! - **Ready**: the benchmark starts from the command line and exits by itself.
//! - **Needs manual start**: the benchmark exists and is good, but lives behind
//!   a menu. Frametimes can still be captured while a person runs it; what
//!   cannot be automated is the starting.
//! - **Missing dependency**: installed, but something it needs is absent.
//! - **Not installed**: absent.
//!
//! Reporting a menu-only benchmark as simply "unavailable" would be a lie of
//! omission — the benchmark is there, and someone willing to click four times
//! can have a very good measurement out of it.

use std::path::Path;

use anyhow::Result;

use super::provider::{Availability, BenchmarkProvider, RunContext, RunOutcome, Source};
use crate::games::{self, DetectedGame};

/// How a title's built-in benchmark is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A command-line flag starts it and the game exits when it finishes.
    CommandLine(&'static str),
    /// It is reached through the game's own menus.
    Menu(&'static str),
}

/// A title this module knows something about.
struct Known {
    /// Steam application id — the only identifier stable across languages,
    /// install paths and library folders.
    app_id: &'static str,
    /// Name for the user. The detected title is preferred when available.
    name: &'static str,
    /// How its benchmark starts.
    entry: Entry,
    /// A library the game needs and does not bundle, if one is known missing.
    missing_library: Option<&'static str>,
}

/// The titles with a built-in benchmark worth driving.
///
/// Keyed by Steam id rather than by folder name, so a library on another disk,
/// a non-English install or a renamed directory all still resolve.
const KNOWN: &[Known] = &[
    Known {
        app_id: "750920",
        name: "Shadow of the Tomb Raider",
        // The Windows build under Proton. Its benchmark is thorough -- it
        // reports CPU and GPU frame rates separately -- but it is reached
        // through Options, and the game exposes no flag to start it.
        entry: Entry::Menu("Options → Display → Run Benchmark"),
        missing_library: None,
    },
    Known {
        app_id: "391220",
        name: "Rise of the Tomb Raider",
        // The Feral Linux port accepts -benchmark, but its launcher window
        // opens first and -nolauncher does not suppress it, so an unattended
        // run stops at a dialog nobody is there to dismiss.
        entry: Entry::Menu("the Feral launcher, then Options → Benchmark"),
        missing_library: None,
    },
    Known {
        app_id: "203160",
        name: "Tomb Raider (2013)",
        entry: Entry::CommandLine("-benchmark"),
        // The Feral port is a 64-bit binary whose bundled lib/ directory holds
        // only 32-bit objects, and the Steam runtime does not supply the
        // missing one either. It cannot start on a current system.
        missing_library: Some("libicui18n.so.51"),
    },
    Known {
        app_id: "1091500",
        name: "Cyberpunk 2077",
        entry: Entry::Menu("Settings → Graphics → Run Benchmark"),
        missing_library: None,
    },
];

/// One installed title with a built-in benchmark.
pub struct GameBenchmark {
    known: &'static Known,
    detected: Option<DetectedGame>,
}

impl GameBenchmark {
    /// Build providers for every known title that is installed.
    #[must_use]
    pub fn detect_all() -> Vec<Self> {
        let installed = games::detect_all();
        KNOWN
            .iter()
            .map(|known| Self {
                known,
                detected: installed
                    .iter()
                    .find(|g| g.app_id.as_deref() == Some(known.app_id))
                    .cloned(),
            })
            .collect()
    }

    /// Whether the library this title is known to need is actually absent.
    ///
    /// Checked rather than assumed: a distribution that ships the compatibility
    /// package would make the game runnable, and hardcoding "broken" would then
    /// be wrong on that machine. Rule: report what this machine has, not what
    /// one machine had.
    fn dependency_missing(&self) -> Option<&'static str> {
        let library = self.known.missing_library?;
        let bundled = self
            .detected
            .as_ref()
            .and_then(|g| g.install_path.as_ref())
            .is_some_and(|p| Path::new(p).join("lib").join(library).exists());
        if bundled {
            return None;
        }
        let on_system = ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu"]
            .iter()
            .any(|dir| Path::new(dir).join(library).exists());
        (!on_system).then_some(library)
    }
}

impl BenchmarkProvider for GameBenchmark {
    fn id(&self) -> &'static str {
        self.known.app_id
    }

    fn name(&self) -> &'static str {
        self.known.name
    }

    fn source(&self) -> Source {
        Source::MangoHud
    }

    fn availability(&self) -> Availability {
        if self.detected.is_none() {
            return Availability::NotInstalled(format!(
                "{} is not installed (Steam app {})",
                self.known.name, self.known.app_id
            ));
        }
        if let Some(library) = self.dependency_missing() {
            return Availability::MissingDependency(format!(
                "{library} is missing and the game does not bundle a usable copy"
            ));
        }
        match self.known.entry {
            Entry::Menu(where_) => Availability::NeedsManualStart(format!(
                "its benchmark is reached through {where_}; frametimes can be \
                 captured while it runs, but starting it cannot be automated"
            )),
            Entry::CommandLine(_) => Availability::Ready,
        }
    }

    fn run(&self, _ctx: &RunContext) -> Result<RunOutcome> {
        // Deliberately not implemented by guessing. A title whose availability
        // is anything but Ready must not be "run" with a fabricated result, and
        // the command-line titles on this list are currently all blocked by a
        // missing dependency, so there is no path here that could produce a
        // real number.
        anyhow::bail!(
            "{} cannot be run unattended: {}",
            self.known.name,
            self.availability()
                .reason()
                .unwrap_or("no automated entry point")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_title_has_an_id_and_a_name() {
        for known in KNOWN {
            assert!(known.app_id.chars().all(|c| c.is_ascii_digit()));
            assert!(!known.name.is_empty());
        }
    }

    #[test]
    fn app_ids_are_unique() {
        let mut ids: Vec<&str> = KNOWN.iter().map(|k| k.app_id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "a duplicated app id would shadow a title");
    }

    #[test]
    fn a_title_that_is_not_installed_says_so_with_its_id() {
        let provider = GameBenchmark {
            known: &KNOWN[0],
            detected: None,
        };
        let availability = provider.availability();
        assert!(!availability.is_ready());
        let reason = availability.reason().unwrap();
        assert!(reason.contains("not installed"));
        assert!(
            reason.contains(KNOWN[0].app_id),
            "the id lets a user find it"
        );
    }

    #[test]
    fn a_menu_only_benchmark_is_reported_as_manual_not_as_absent() {
        let sottr = KNOWN.iter().find(|k| k.app_id == "750920").unwrap();
        let provider = GameBenchmark {
            known: sottr,
            detected: Some(DetectedGame {
                name: sottr.name.into(),
                source: crate::games::Source::Steam,
                app_id: Some(sottr.app_id.into()),
                install_path: None,
                executables: vec!["SOTTR.exe".into()],
                cover: None,
                launch_command: None,
            }),
        };
        let availability = provider.availability();
        assert!(matches!(availability, Availability::NeedsManualStart(_)));
        // The distinction that matters: the benchmark exists.
        let reason = availability.reason().unwrap();
        assert!(reason.contains("Options"));
        assert!(!reason.contains("not installed"));
    }

    #[test]
    fn running_an_unautomatable_title_errors_rather_than_inventing_a_number() {
        let provider = GameBenchmark {
            known: &KNOWN[0],
            detected: None,
        };
        let ctx = RunContext {
            output_dir: std::env::temp_dir(),
            duration: std::time::Duration::from_secs(1),
        };
        let error = provider.run(&ctx).unwrap_err();
        assert!(error.to_string().contains("cannot be run unattended"));
    }

    #[test]
    fn detection_reports_one_entry_per_known_title() {
        // Whatever is installed here, every known title gets a row, so the UI
        // can show "not installed" rather than silently omitting it.
        let providers = GameBenchmark::detect_all();
        assert_eq!(providers.len(), KNOWN.len());
        for provider in &providers {
            let availability = provider.availability();
            if !availability.is_ready() {
                assert!(availability.reason().is_some_and(|r| !r.is_empty()));
            }
        }
    }
}
