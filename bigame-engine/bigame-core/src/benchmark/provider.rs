//! Benchmark workloads, and whether each one can be measured here.
//!
//! A provider knows one workload and answers one question: can it run on this
//! machine, and if not, what is missing. "Not installed", "installed but
//! missing a dependency" and "needs a person to start it" lead to different
//! things the UI should say, so availability is an answer of its own rather
//! than a boolean.
//!
//! Measuring is done elsewhere: the Measure dialog captures frametimes with
//! `MangoHud` ([`crate::booster::measure`]), and the benchmark scripts record
//! whole sessions.

use std::path::{Path, PathBuf};

/// Whether a benchmark can be run here, and if not, what is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Ready to run.
    Ready,
    /// The workload itself is absent. Carries what to install.
    NotInstalled(String),
    /// Present, but something it needs is not.
    MissingDependency(String),
    /// Present and installed, but cannot be driven without a person.
    ///
    /// Several commercial games have an excellent built-in benchmark reachable
    /// only from a menu. Saying so is more useful than pretending the benchmark
    /// does not exist.
    NeedsManualStart(String),
}

impl Availability {
    /// Whether it can be run now.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// What to tell the user, when it is not ready.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::NotInstalled(s) | Self::MissingDependency(s) | Self::NeedsManualStart(s) => {
                Some(s)
            }
        }
    }
}

/// One benchmarkable workload.
pub trait BenchmarkProvider: Send + Sync {
    /// Stable identifier, used in result files and on disk.
    fn id(&self) -> &'static str;

    /// Name for the user.
    fn name(&self) -> &'static str;

    /// Whether this can run here, and what is missing if not.
    fn availability(&self) -> Availability;
}

// ── SuperTuxKart ─────────────────────────────────────────────────────────────

/// `SuperTuxKart`'s built-in benchmark.
///
/// The best automated workload found on this machine, for four reasons: it
/// replays a recorded lap rather than simulating one, so every run renders the
/// same frames; it exits by itself; it writes a per-frame CSV; and it is free
/// software, so a result can be reproduced by anyone.
///
/// One caveat matters enough to encode here. Out of the box STK runs with
/// vsync on and `max_fps=120`, and a capped workload cannot show a difference
/// between two configurations no matter how large that difference is. The
/// provider reports that as a dependency problem rather than silently producing
/// a meaningless comparison.
pub struct SuperTuxKart {
    root: Option<PathBuf>,
}

impl Default for SuperTuxKart {
    fn default() -> Self {
        Self::new()
    }
}

impl SuperTuxKart {
    /// Locate an installation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Self::find_installation(),
        }
    }

    /// Where `SuperTuxKart` lives, if it does.
    ///
    /// A packaged install is preferred; a self-contained one unpacked under the
    /// user's home is accepted, since that is how the current release is often
    /// distributed.
    fn find_installation() -> Option<PathBuf> {
        if let Some(binary) = crate::capabilities::which("supertuxkart") {
            return Some(binary);
        }
        let home = std::env::var("HOME").ok()?;
        let home = Path::new(&home);
        for base in [
            home.join("Downloads"),
            home.join("Games"),
            home.join("Jogos"),
        ] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin/supertuxkart");
                if candidate.is_file() {
                    return Some(candidate);
                }
                // One directory deeper — releases are often unpacked into a
                // folder of their own inside a downloads folder.
                let Ok(inner) = std::fs::read_dir(entry.path()) else {
                    continue;
                };
                for sub in inner.flatten() {
                    let candidate = sub.path().join("bin/supertuxkart");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    /// STK's configuration directory for the 0.10 profile format.
    #[must_use]
    pub fn config_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })?;
        let dir = base.join("supertuxkart").join("config-0.10");
        dir.is_dir().then_some(dir)
    }

    /// Whether the frame rate is capped, which would make a comparison useless.
    ///
    /// Returns `None` when there is no configuration to read yet — STK writes
    /// one on first launch.
    #[must_use]
    pub fn frame_cap(config: &Path) -> Option<String> {
        let content = std::fs::read_to_string(config.join("config.xml")).ok()?;
        let vsync = xml_attribute(&content, "swap-interval-vsync");
        let max_fps = xml_attribute(&content, "max_fps");
        match (vsync.as_deref(), max_fps.as_deref()) {
            (Some(v), _) if v != "0" => Some(format!("vsync is on (swap-interval-vsync={v})")),
            (_, Some(f)) if f.parse::<u32>().is_ok_and(|n| n <= 300) => {
                Some(format!("frame rate is capped at {f}"))
            }
            _ => None,
        }
    }
}

/// Read `name="value"` out of XML-ish text.
fn xml_attribute(content: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = content.find(&needle)? + needle.len();
    let rest = &content[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

impl BenchmarkProvider for SuperTuxKart {
    fn id(&self) -> &'static str {
        "supertuxkart"
    }

    fn name(&self) -> &'static str {
        "SuperTuxKart"
    }

    fn availability(&self) -> Availability {
        let Some(_) = &self.root else {
            return Availability::NotInstalled("supertuxkart".into());
        };
        let Some(config) = Self::config_dir() else {
            // The configuration appears on first launch; the benchmark will
            // create it, so this is not fatal.
            return Availability::Ready;
        };
        if let Some(cap) = Self::frame_cap(&config) {
            // Out of the box STK writes vsync on and max_fps 120 on its first
            // launch, so every new install lands here; the limit is not in
            // the game's menus, so say exactly what to change and where.
            return Availability::MissingDependency(format!(
                "{cap} — a capped workload cannot show a difference between two \
                 configurations, however large it is. With SuperTuxKart closed, set \
                 swap-interval-vsync=\"0\" and max_fps=\"1000\" in {}",
                config.join("config.xml").display()
            ));
        }
        Availability::Ready
    }
}

// ── Unigine Superposition ────────────────────────────────────────────────────

/// Unigine Superposition, the synthetic GPU benchmark.
///
/// A real GPU workload over a real scene, which makes it far better evidence
/// than any spinning-cube loop. Two things limit it here.
///
/// **It is started by hand.** The `superposition_cli` binary ships with the
/// free edition but does nothing: it returns success without running anything,
/// even for an XML file that does not exist. Unattended runs are a Pro-edition
/// feature, so the free edition is a GUI benchmark and nothing else.
///
/// **Its packaging has been known to install unreadable.** The Arch package
/// installed `/opt/unigine-superposition` with every directory `drwxr-x---`
/// and root-owned, so the launcher failed at `cd` with "Failed to change
/// working directory" before reaching any graphics code. That is checked here
/// rather than assumed, because it is a per-machine condition a package update
/// can reintroduce, and because "installed but unreadable" needs a different
/// message from "not installed".
pub struct Superposition {
    root: Option<PathBuf>,
}

impl Default for Superposition {
    fn default() -> Self {
        Self::new()
    }
}

impl Superposition {
    /// Locate an installation.
    #[must_use]
    pub fn new() -> Self {
        let candidates = [
            PathBuf::from("/opt/unigine-superposition"),
            PathBuf::from("/opt/Unigine/Superposition"),
        ];
        Self {
            root: candidates.into_iter().find(|p| p.is_dir()),
        }
    }

    /// Whether the engine directory can actually be entered and read.
    ///
    /// The launcher's first act is to change into `bin/`, so a directory the
    /// user cannot traverse stops it before anything else can go wrong.
    fn unreadable_part(root: &Path) -> Option<PathBuf> {
        ["bin", "data"]
            .iter()
            .map(|name| root.join(name))
            .find(|dir| dir.is_dir() && std::fs::read_dir(dir).is_err())
    }
}

impl BenchmarkProvider for Superposition {
    fn id(&self) -> &'static str {
        "unigine-superposition"
    }

    fn name(&self) -> &'static str {
        "Unigine Superposition"
    }

    fn availability(&self) -> Availability {
        let Some(root) = &self.root else {
            return Availability::NotInstalled("unigine-superposition".into());
        };
        if let Some(dir) = Self::unreadable_part(root) {
            return Availability::MissingDependency(format!(
                "{} cannot be read by this user, so the launcher stops at \
                 \"Failed to change working directory\"; \
                 `sudo chmod -R a+rX {}` fixes it",
                dir.display(),
                root.display()
            ));
        }
        Availability::NeedsManualStart(
            "its command-line mode is a Pro-edition feature -- the free \
             edition's superposition_cli exits without running anything -- so \
             the benchmark has to be started from the launcher window"
                .into(),
        )
    }
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Every provider this build knows about.
///
/// Installed games come first: a built-in benchmark over a real scene is better
/// evidence about gaming performance than any synthetic workload, even when it
/// has to be started by hand.
#[must_use]
pub fn all() -> Vec<Box<dyn BenchmarkProvider>> {
    let mut providers: Vec<Box<dyn BenchmarkProvider>> = super::games::GameBenchmark::detect_all()
        .into_iter()
        .map(|g| Box::new(g) as Box<dyn BenchmarkProvider>)
        .collect();
    providers.push(Box::new(Superposition::new()));
    providers.push(Box::new(SuperTuxKart::new()));
    providers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_distinguishes_the_three_reasons() {
        assert!(Availability::Ready.is_ready());
        assert_eq!(Availability::Ready.reason(), None);

        let missing = Availability::NotInstalled("supertuxkart".into());
        assert!(!missing.is_ready());
        assert_eq!(missing.reason(), Some("supertuxkart"));

        // "Installed but capped" and "not installed" need different messages.
        let capped = Availability::MissingDependency("vsync is on".into());
        assert!(!capped.is_ready());
        assert_ne!(capped, missing);

        let manual = Availability::NeedsManualStart("benchmark is in the menu".into());
        assert!(!manual.is_ready());
    }

    #[test]
    fn xml_attributes_are_read() {
        let xml = r#"<config max_fps="1000" swap-interval-vsync="0" other="x" />"#;
        assert_eq!(xml_attribute(xml, "max_fps").as_deref(), Some("1000"));
        assert_eq!(
            xml_attribute(xml, "swap-interval-vsync").as_deref(),
            Some("0")
        );
        assert_eq!(xml_attribute(xml, "absent"), None);
    }

    #[test]
    fn a_capped_configuration_is_refused_with_the_reason() {
        let dir = std::env::temp_dir().join(format!(
            "bigame_stk_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // vsync on — the default, and useless for comparison.
        std::fs::write(
            dir.join("config.xml"),
            r#"<config max_fps="1000" swap-interval-vsync="1" />"#,
        )
        .unwrap();
        assert!(SuperTuxKart::frame_cap(&dir).unwrap().contains("vsync"));

        // vsync off but a low cap — equally useless.
        std::fs::write(
            dir.join("config.xml"),
            r#"<config max_fps="120" swap-interval-vsync="0" />"#,
        )
        .unwrap();
        assert!(
            SuperTuxKart::frame_cap(&dir)
                .unwrap()
                .contains("capped at 120")
        );

        // Uncapped: usable.
        std::fs::write(
            dir.join("config.xml"),
            r#"<config max_fps="1000" swap-interval-vsync="0" />"#,
        )
        .unwrap();
        assert_eq!(SuperTuxKart::frame_cap(&dir), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_engine_directory_is_reported_with_the_fix() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("bigame_super_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();

        // Readable: nothing to report.
        assert_eq!(Superposition::unreadable_part(&root), None);

        // The packaging defect this exists to catch: the launcher's first act
        // is to change into bin/, so a directory it cannot enter stops it
        // before any graphics code runs.
        std::fs::set_permissions(root.join("bin"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let blocked = Superposition::unreadable_part(&root);

        // Running the suite as root would defeat the check, so only assert the
        // message when the permission actually bites.
        if let Some(dir) = blocked {
            assert!(dir.ends_with("bin"));
            let provider = Superposition {
                root: Some(root.clone()),
            };
            let availability = provider.availability();
            assert!(matches!(availability, Availability::MissingDependency(_)));
            let reason = availability.reason().unwrap();
            // The message has to carry the remedy: this is a one-command fix
            // and a user who is only told "blocked" cannot act on it.
            assert!(reason.contains("chmod -R a+rX"), "{reason}");
            assert!(
                reason.contains("Failed to change working directory"),
                "{reason}"
            );
        }

        let _ = std::fs::set_permissions(root.join("bin"), std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn superposition_without_an_install_says_not_installed() {
        let provider = Superposition { root: None };
        let availability = provider.availability();
        assert!(matches!(availability, Availability::NotInstalled(_)));
        assert!(!availability.is_ready());
    }

    #[test]
    fn a_readable_superposition_still_needs_a_person() {
        // The free edition ships superposition_cli, but it returns success
        // without running anything, so "installed and readable" still is not
        // "can be measured unattended".
        let root = std::env::temp_dir().join(format!("bigame_super_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();

        let provider = Superposition {
            root: Some(root.clone()),
        };
        let availability = provider.availability();
        assert!(matches!(availability, Availability::NeedsManualStart(_)));
        assert!(availability.reason().unwrap().contains("Pro-edition"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_registry_reports_this_machine_honestly() {
        for provider in all() {
            assert!(!provider.id().is_empty());
            assert!(!provider.name().is_empty());
            // Whatever the answer, it must carry a reason when not ready.
            let availability = provider.availability();
            if !availability.is_ready() {
                assert!(availability.reason().is_some_and(|r| !r.is_empty()));
            }
        }
    }
}
