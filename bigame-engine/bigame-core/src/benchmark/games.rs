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

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;

use super::native::{self, NativeRun};
use super::provider::{Availability, BenchmarkProvider, RunContext, RunOutcome, Source};
use crate::games::{self, DetectedGame};

/// How a title writes its benchmark results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// `*_frametimes_*.txt` beside a summary `.txt`.
    Crystal,
    /// A `benchmark_*` directory holding `frames.csv` and `summary.json`.
    Cyberpunk,
}

/// Where a title writes its results, relative to the Windows user directory of
/// its Proton prefix (`drive_c/users/steamuser`).
#[derive(Debug, Clone, Copy)]
struct Results {
    dir: &'static str,
    format: Format,
}

/// A title this module knows something about.
struct Known {
    /// Steam application id — the only identifier stable across languages,
    /// install paths and library folders.
    app_id: &'static str,
    /// Name for the user. The detected title is preferred when available.
    name: &'static str,
    /// Where its benchmark is reached from.
    ///
    /// A plain string rather than an enum of entry points: every title found
    /// here needs a person to start it, and inventing a `CommandLine` variant
    /// nothing uses would be describing a capability this machine has not
    /// actually got. When a title turns up that can be started unattended, the
    /// distinction can be added along with it.
    reached_by: &'static str,
    /// A library the game needs and does not bundle, if one is known missing.
    missing_library: Option<&'static str>,
    /// Where its own per-frame results appear, when that has been verified by
    /// finding them there. `None` means unverified, not absent.
    results: Option<Results>,
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
        // reports CPU and GPU frame rates separately -- and it ends on a
        // results screen whose [R] key runs it again without reloading, which
        // is what makes a whole alternating session possible from one launch
        // (scripts/bench-game.sh). The first pass still has to be started from
        // Options: the game exposes no flag for it.
        reached_by: "Options → Display → Run Benchmark",
        missing_library: None,
        results: Some(Results {
            dir: "Documents/Shadow of the Tomb Raider",
            format: Format::Crystal,
        }),
    },
    Known {
        app_id: "391220",
        name: "Rise of the Tomb Raider",
        // Installed here as the Windows build under Proton. It writes the
        // same frametime format as Shadow, one file per scene -- seen in its
        // prefix on the reference machine after a run on 2026-09-24.
        reached_by: "Options → Graphics → Run Benchmark",
        missing_library: None,
        results: Some(Results {
            dir: "Documents/Rise of the Tomb Raider",
            format: Format::Crystal,
        }),
    },
    Known {
        app_id: "203160",
        name: "Tomb Raider (2013)",
        // The Feral port accepts -benchmark, but reaching it is not
        // straightforward on a current system and was not achieved here.
        //
        // The native binary is 32-bit and bundles its dependencies in
        // lib/i686, including the ICU libraries an earlier note wrongly
        // recorded as missing. Launched inside the Steam scout runtime it gets
        // as far as initialising -- but only with its own libcurl preloaded,
        // because the runtime pins a libcurl lacking the CURL_OPENSSL_4
        // version the binary needs. It then aborts with
        // `basic_filebuf::underflow` reading some file, which was not
        // identified.
        //
        // Separately, this Steam installation is configured to run the title
        // through Proton (TombRaider.exe) rather than the native build, so the
        // native path is not the one Steam would take anyway.
        reached_by: "Steam, which runs it through Proton here; the native build's \
             -benchmark path aborts during start-up for reasons not yet found",
        missing_library: None,
        results: None,
    },
    Known {
        app_id: "1091500",
        name: "Cyberpunk 2077",
        reached_by: "Settings → Graphics → Run Benchmark",
        missing_library: None,
        results: Some(Results {
            dir: "Documents/CD Projekt Red/Cyberpunk 2077/benchmarkResults",
            format: Format::Cyberpunk,
        }),
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

impl GameBenchmark {
    /// Where this title writes its benchmark results on this machine.
    ///
    /// Derived from the install path, never searched for: Steam leaves the old
    /// `compatdata/<id>` behind when a game moves between libraries, and on the
    /// reference machine a stale prefix in the home library shadows the live
    /// one on the games disk. The live prefix is the one in the same library
    /// as the game, i.e. `<library>/steamapps/compatdata/<id>`.
    #[must_use]
    pub fn results_dir(&self) -> Option<PathBuf> {
        let results = self.known.results?;
        let install = self.detected.as_ref()?.install_path.as_ref()?;
        // <library>/steamapps/common/<dir> → <library>/steamapps
        let steamapps = install.parent()?.parent()?;
        Some(
            steamapps
                .join("compatdata")
                .join(self.known.app_id)
                .join("pfx/drive_c/users/steamuser")
                .join(results.dir),
        )
    }

    /// The newest run of this title's own benchmark finished at or after
    /// `since`, read from the files the game wrote.
    ///
    /// This is how a menu-only benchmark becomes measurable: a person starts
    /// it, and the result is picked up without an overlay or any change to
    /// the game's launch options.
    ///
    /// # Errors
    /// Returns an error when the results directory exists but a run in it
    /// cannot be parsed.
    pub fn collect_since(&self, since: SystemTime) -> Result<Option<NativeRun>> {
        let (Some(dir), Some(results)) = (self.results_dir(), self.known.results) else {
            return Ok(None);
        };
        if !dir.is_dir() {
            return Ok(None);
        }
        match results.format {
            // Every scene file of the run: Rise writes one per scene.
            Format::Crystal => {
                let files = native::all_since(&dir, |n| n.contains("_frametimes_"), since)?;
                if files.is_empty() {
                    Ok(None)
                } else {
                    native::read_crystal_run(&files).map(Some)
                }
            }
            Format::Cyberpunk => {
                native::newest_since(&dir, |n| n.starts_with("benchmark_"), since)?
                    .map(|d| native::read_cyberpunk(&d))
                    .transpose()
            }
        }
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
        if self.known.results.is_some() {
            Source::NativeFrameTimes
        } else {
            Source::MangoHud
        }
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
        let capture = if self.known.results.is_some() {
            "the game records every frame itself, and the result is read from its own files"
        } else {
            "frametimes can be captured while it runs"
        };
        Availability::NeedsManualStart(format!(
            "its benchmark is reached through {}; {capture}, but starting it cannot be automated",
            self.known.reached_by
        ))
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

    fn installed(app_id: &str, install: &str) -> GameBenchmark {
        let known = KNOWN.iter().find(|k| k.app_id == app_id).unwrap();
        GameBenchmark {
            known,
            detected: Some(DetectedGame {
                name: known.name.into(),
                source: crate::games::Source::Steam,
                app_id: Some(app_id.into()),
                install_path: Some(install.into()),
                executables: vec![],
                cover: None,
                launch_command: None,
            }),
        }
    }

    #[test]
    fn the_prefix_is_the_one_beside_the_game_not_the_first_found() {
        let sottr = installed(
            "750920",
            "/run/media/u/Games/steamapps/common/Shadow of the Tomb Raider",
        );
        assert_eq!(
            sottr.results_dir().unwrap(),
            Path::new(
                "/run/media/u/Games/steamapps/compatdata/750920/pfx/drive_c/users/steamuser/\
                 Documents/Shadow of the Tomb Raider"
            )
        );
    }

    #[test]
    fn an_unverified_location_is_not_guessed() {
        let tr2013 = installed("203160", "/g/steamapps/common/Tomb Raider");
        assert!(tr2013.results_dir().is_none());
        assert!(matches!(tr2013.source(), Source::MangoHud));
    }

    #[test]
    fn a_finished_run_is_collected_from_the_games_own_files() {
        let root = std::env::temp_dir().join(format!("bgm-games-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let install = root.join("steamapps/common/Cyberpunk 2077");
        std::fs::create_dir_all(&install).unwrap();
        let cp = installed("1091500", install.to_str().unwrap());
        let run_dir = cp
            .results_dir()
            .unwrap()
            .join("benchmark_2026-09-23_19-55-25");
        std::fs::create_dir_all(&run_dir).unwrap();
        let csv: String = std::iter::once("Frame index, Frame time (ms)\n".to_owned())
            .chain((0..200).map(|i| format!("{i}, 20.0\n")))
            .collect();
        std::fs::write(run_dir.join("frames.csv"), csv).unwrap();
        std::fs::write(
            run_dir.join("summary.json"),
            r#"{"Data":{"averageFps":50.0,"presetName":"High","frameGenerationType":0}}"#,
        )
        .unwrap();

        let run = cp.collect_since(SystemTime::UNIX_EPOCH).unwrap().unwrap();
        let stats = run.capture.stats().unwrap();
        assert!((stats.avg_fps - 50.0).abs() < 1e-9);
        assert_eq!(run.reported_avg_fps, Some(50.0));
        assert!(cp.availability().reason().unwrap().contains("own files"));
        let _ = std::fs::remove_dir_all(&root);
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
