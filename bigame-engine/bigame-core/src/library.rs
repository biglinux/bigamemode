//! The game library: installed games, each with the profile that tunes it,
//! if one exists.
//!
//! Games and profiles are two different things and stay so. A game is what
//! [`crate::games`] found on disk; a profile is a file falcond reads. The
//! library is built from the games, and profiles are looked up for them —
//! never the other way round, because falcond ships profiles for titles that
//! are not installed, and a profile the user wrote by hand proves nothing
//! about the game either. Profiles that match no installed game are kept
//! (the game may be installed again) and reported separately.

use crate::games::DetectedGame;
use crate::profiles::ProfileRef;

/// An installed game and its profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The game.
    pub game: DetectedGame,
    /// The profile for one of the game's process names, if any.
    pub profile: Option<ProfileRef>,
}

impl Entry {
    /// The process name a profile for this game is, or would be, keyed on:
    /// the matched profile's, otherwise the game's best executable.
    #[must_use]
    pub fn key(&self) -> &str {
        self.profile
            .as_ref()
            .map_or_else(|| self.game.profile_key(), |p| p.name.as_str())
    }
}

/// What the library page shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    /// Installed games, sorted by title.
    pub games: Vec<Entry>,
    /// The user's profiles that match no installed game: written by hand,
    /// imported, or left from a game since removed. Not the ones falcond
    /// ships, which are its own business.
    pub unmatched_profiles: Vec<ProfileRef>,
}

/// Scan the machine: every installed game, and the profiles on disk.
#[must_use]
pub fn scan() -> Library {
    assemble(crate::games::detect_all(), crate::profiles::index())
}

/// Pair each game with its profile.
///
/// A game matches the first profile named after any of its candidate
/// executables, in rank order, so a profile the user keyed on the second
/// candidate still counts. A game with no executable found matches on its
/// title, which is what its key would fall back to.
#[must_use]
pub fn assemble(games: Vec<DetectedGame>, profiles: Vec<ProfileRef>) -> Library {
    let mut matched = vec![false; profiles.len()];
    let entries = games
        .into_iter()
        .map(|game| {
            let keys: Vec<&str> = if game.has_real_executable() {
                game.executables.iter().map(String::as_str).collect()
            } else {
                vec![game.name.as_str()]
            };
            let found = keys
                .iter()
                .find_map(|key| profiles.iter().position(|p| p.matches(key)));
            let profile = found.map(|i| {
                matched[i] = true;
                profiles[i].clone()
            });
            Entry { game, profile }
        })
        .collect();
    let unmatched_profiles = profiles
        .into_iter()
        .zip(matched)
        .filter(|(p, hit)| !hit && !p.system)
        .map(|(p, _)| p)
        .collect();
    Library {
        games: entries,
        unmatched_profiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::Source;

    fn game(name: &str, executables: &[&str]) -> DetectedGame {
        DetectedGame {
            name: name.into(),
            source: Source::Steam,
            app_id: None,
            install_path: None,
            executables: executables.iter().map(|e| (*e).to_owned()).collect(),
            launch_file: None,
            cover: None,
            icon: None,
            launch_command: None,
            launcher: None,
        }
    }

    fn profile(stem: &str, name: &str, system: bool) -> ProfileRef {
        ProfileRef {
            stem: stem.into(),
            name: name.into(),
            system,
        }
    }

    #[test]
    fn a_profile_without_an_installed_game_is_kept_but_is_not_a_game() {
        let library = assemble(
            vec![game("ARC Raiders", &["PioneerGame.exe"])],
            vec![
                profile("cs2", "cs2", true),
                profile("Cyberpunk2077.exe", "Cyberpunk2077.exe", false),
            ],
        );
        assert_eq!(library.games.len(), 1);
        assert_eq!(library.games[0].profile, None);
        // The user's profile is reported, falcond's is not.
        assert_eq!(
            library.unmatched_profiles,
            vec![profile("Cyberpunk2077.exe", "Cyberpunk2077.exe", false)]
        );
    }

    #[test]
    fn an_installed_game_with_a_profile_is_one_entry_with_that_profile() {
        let library = assemble(
            vec![game(
                "Cyberpunk 2077",
                &["Cyberpunk2077.exe", "REDprelauncher.exe"],
            )],
            // falcond's file is named after the game, not the process.
            vec![profile("cyberpunk2077", "Cyberpunk2077.exe", true)],
        );
        assert_eq!(library.games.len(), 1);
        let entry = &library.games[0];
        assert_eq!(
            entry.profile.as_ref().map(|p| p.stem.as_str()),
            Some("cyberpunk2077")
        );
        assert_eq!(entry.key(), "Cyberpunk2077.exe");
        assert!(library.unmatched_profiles.is_empty());
    }

    #[test]
    fn a_profile_on_a_lower_ranked_executable_still_counts() {
        let library = assemble(
            vec![game(
                "Cyberpunk 2077",
                &["Cyberpunk2077.exe", "REDprelauncher.exe"],
            )],
            vec![profile("REDprelauncher.exe", "REDprelauncher.exe", false)],
        );
        assert_eq!(library.games[0].key(), "REDprelauncher.exe");
        assert!(library.unmatched_profiles.is_empty());
    }

    #[test]
    fn a_game_without_an_executable_matches_a_profile_on_its_title() {
        let library = assemble(
            vec![game("Some Game", &[])],
            vec![profile("Some Game", "Some Game", false)],
        );
        assert!(library.games[0].profile.is_some());
    }

    #[test]
    fn a_game_without_a_profile_keys_on_its_executable() {
        let library = assemble(vec![game("ARC Raiders", &["PioneerGame.exe"])], vec![]);
        assert_eq!(library.games[0].key(), "PioneerGame.exe");
    }
}
