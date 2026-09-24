//! Moving profiles written by older BiGame-mode versions onto the new model.
//!
//! The profiles that ship with falcond (`profiles/`, `profiles/handheld/`,
//! `profiles/htpc/`) belong to the `falcond-profiles` package and are never
//! touched. Only `profiles/user/` is considered, and within it only files an
//! older BiGame-mode demonstrably wrote — recognisable by fields falcond does
//! not define (`fg_multiplier`, `cpu_governor`, `enabled`, …), which that
//! version emitted and nothing else does.
//!
//! Those profiles have two defects:
//!
//! 1. **Keyed on the game's title**, which falcond can never match, because
//!    falcond matches process names. `Arc Raiders.conf` with `name = "Arc
//!    Raiders"` does nothing; the process is `PioneerGame.exe`.
//! 2. **Fields falcond ignores**, some of them controls that looked like they
//!    worked: a per-game `cpu_governor` that nothing applied, and an `enabled`
//!    flag whose `false` left the profile active in falcond.
//!
//! The plan re-keys what can be resolved to an installed game's real
//! executable, keeping only falcond's fields, and reports the rest rather than
//! deleting it. Nothing is changed until [`plan`]'s result is executed, and
//! every file is copied to a backup first.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::games::DetectedGame;

/// The fields falcond 2.0.2 reads from a profile. Anything else is ignored by
/// falcond, and is dropped on migration.
pub const FALCOND_FIELDS: &[&str] = &[
    "name",
    "performance_mode",
    "scx_sched",
    "scx_sched_props",
    "vcache_mode",
    "idle_inhibit",
];

/// Fields only BiGame-mode ever wrote: their presence identifies its files.
const BIGAME_FIELDS: &[&str] = &[
    "fg_multiplier",
    "fg_flow_scale",
    "fg_perf_mode",
    "fg_hdr",
    "fg_present_mode",
    "cpu_governor",
    "scx_custom_flags",
    "enabled",
    "gamescope",
    "gamescope_mode",
];

/// What to do with one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Re-key onto the game's real process, keeping only falcond's fields.
    Rekey {
        /// The file now.
        file: PathBuf,
        /// Its `name` now (a title).
        from: String,
        /// The process falcond will match.
        to: String,
        /// The profile as it will be written.
        content: String,
    },
    /// Already keyed on a process; only the fields falcond ignores go.
    Clean {
        /// The file.
        file: PathBuf,
        /// Its `name`, kept.
        name: String,
        /// The profile as it will be written.
        content: String,
    },
    /// Not written by BiGame-mode, or already clean: left alone.
    Keep {
        /// The file.
        file: PathBuf,
        /// Why.
        reason: String,
    },
    /// Written by BiGame-mode, but no installed game matches its name.
    /// Reported, not deleted: the game may be on a disk that is not mounted.
    Unresolved {
        /// The file.
        file: PathBuf,
        /// Its `name`.
        name: String,
    },
}

/// `key = value` pairs of a profile, in order, comments dropped.
fn fields(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect()
}

/// The profile with only falcond's fields, and `name` set to `name`.
#[must_use]
pub fn falcond_only(content: &str, name: &str) -> String {
    let mut out = format!("name = \"{}\"\n", name.replace('"', ""));
    for (k, v) in fields(content) {
        if k != "name" && FALCOND_FIELDS.contains(&k.as_str()) {
            let _ = writeln!(out, "{k} = {v}");
        }
    }
    out
}

/// Whether a name is already a process name rather than a title.
fn looks_like_process(name: &str, installed: &[DetectedGame]) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        || installed
            .iter()
            .any(|g| g.executables.iter().any(|e| e.eq_ignore_ascii_case(name)))
}

/// Decide what to do with one file.
#[must_use]
pub fn plan_file(file: &Path, content: &str, installed: &[DetectedGame]) -> Action {
    let pairs = fields(content);
    let written_by_bigame = pairs
        .iter()
        .any(|(k, _)| BIGAME_FIELDS.contains(&k.as_str()));
    let Some(name) = crate::running::profile_name_field(content) else {
        return Action::Keep {
            file: file.to_path_buf(),
            reason: "has no name field".into(),
        };
    };
    if !written_by_bigame {
        return Action::Keep {
            file: file.to_path_buf(),
            reason: "not written by BiGame-mode".into(),
        };
    }
    if looks_like_process(&name, installed) {
        return Action::Clean {
            file: file.to_path_buf(),
            content: falcond_only(content, &name),
            name,
        };
    }
    let game = installed
        .iter()
        .find(|g| g.name.eq_ignore_ascii_case(&name) && g.has_real_executable());
    match game {
        Some(g) => Action::Rekey {
            file: file.to_path_buf(),
            to: g.profile_key().to_owned(),
            content: falcond_only(content, g.profile_key()),
            from: name,
        },
        None => Action::Unresolved {
            file: file.to_path_buf(),
            name,
        },
    }
}

/// Plan the migration of every file in `user_dir`.
#[must_use]
pub fn plan(user_dir: &Path, installed: &[DetectedGame]) -> Vec<Action> {
    let Ok(entries) = std::fs::read_dir(user_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "conf"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| {
            let content = std::fs::read_to_string(f).ok()?;
            Some(plan_file(f, &content, installed))
        })
        .collect()
}

/// Copy every profile in `user_dir` into a new timestamped directory under
/// `backup_root`, and return it.
///
/// # Errors
/// Returns an error if anything could not be copied. A migration must not
/// start without a complete backup.
pub fn backup(user_dir: &Path, backup_root: &Path) -> anyhow::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dest = backup_root.join(format!("profiles-{stamp}"));
    std::fs::create_dir_all(&dest)?;
    for entry in std::fs::read_dir(user_dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "conf") {
            std::fs::copy(&path, dest.join(entry.file_name()))?;
        }
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::Source;

    fn game(name: &str, exe: &str) -> DetectedGame {
        DetectedGame {
            name: name.into(),
            source: Source::Steam,
            app_id: Some("1".into()),
            install_path: None,
            executables: vec![exe.into()],
            cover: None,
            launch_command: None,
        }
    }

    /// The file an older BiGame-mode wrote on the reference machine.
    const ARC: &str = "name = \"Arc Raiders\"\nperformance_mode = true\nscx_sched = none\nscx_sched_props = default\nvcache_mode = none\nidle_inhibit = false\ncpu_governor = \"\"\nscx_custom_flags = \"\"\nenabled = true\nfg_multiplier = 1\nfg_flow_scale = 100\nfg_perf_mode = false\n";

    #[test]
    fn a_title_keyed_profile_is_rekeyed_onto_the_real_process() {
        let installed = [game("ARC Raiders", "PioneerGame.exe")];
        let action = plan_file(Path::new("/p/user/Arc Raiders.conf"), ARC, &installed);
        let Action::Rekey { to, content, .. } = action else {
            panic!("expected a rekey, got {action:?}");
        };
        assert_eq!(to, "PioneerGame.exe");
        assert!(content.starts_with("name = \"PioneerGame.exe\"\n"));
        for dropped in ["cpu_governor", "enabled", "fg_", "scx_custom_flags"] {
            assert!(
                !content.contains(dropped),
                "{dropped} is not a falcond field"
            );
        }
        assert!(
            content.contains("vcache_mode = none"),
            "an explicit value survives"
        );
    }

    #[test]
    fn falconds_own_and_hand_written_profiles_are_left_alone() {
        let upstream = "name = \"Cyberpunk2077.exe\"\nscx_sched = none\nperformance_mode = true\n";
        assert!(matches!(
            plan_file(Path::new("/p/user/cp.conf"), upstream, &[]),
            Action::Keep { .. }
        ));
    }

    #[test]
    fn a_process_keyed_profile_is_only_cleaned() {
        let content = "name = \"cs2\"\nperformance_mode = true\nenabled = false\n";
        let installed = [game("Counter-Strike 2", "cs2")];
        let Action::Clean { name, content, .. } =
            plan_file(Path::new("/p/u/cs2.conf"), content, &installed)
        else {
            panic!("expected clean");
        };
        assert_eq!(name, "cs2");
        assert_eq!(content, "name = \"cs2\"\nperformance_mode = true\n");
    }

    #[test]
    fn an_unmatched_title_is_reported_not_deleted() {
        let action = plan_file(Path::new("/p/u/x.conf"), ARC, &[]);
        assert!(matches!(action, Action::Unresolved { .. }), "{action:?}");
    }

    #[test]
    fn the_backup_holds_every_file() {
        let root = std::env::temp_dir().join(format!("bgm-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let user = root.join("user");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("Arc Raiders.conf"), ARC).unwrap();
        std::fs::write(user.join("notes.txt"), "x").unwrap();
        let dest = backup(&user, &root.join("backup")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("Arc Raiders.conf")).unwrap(),
            ARC
        );
        assert!(!dest.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
