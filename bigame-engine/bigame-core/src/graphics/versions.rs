//! Which `OptiScaler` release a game gets, and whether a newer one is offered.
//!
//! Three policies ([`VersionPolicy`]): the release BiGame-mode was tested
//! with (the default), the latest stable release, or one version pinned by
//! the user. A version that works for a game is never replaced behind the
//! user's back: a newer release is *offered* — Update, Skip, Keep this
//! version — and an update keeps the previous version one click away.
//!
//! What is known about releases comes from the project's GitHub releases,
//! held to the same checks as a download ([`optiscaler::parse_releases`]):
//! stable, one archive, a published SHA-256. The list is cached beside the
//! downloads and refreshed at most once a day, and only when a page that
//! needs it is open — never during a game's launch.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::config::VersionPolicy;
use super::manifest::Source;
use super::optiscaler::{self, Release, compare_versions};

/// The GitHub API list of `OptiScaler` releases.
pub const RELEASES_API: &str =
    "https://api.github.com/repos/optiscaler/OptiScaler/releases?per_page=30";

/// Largest API response accepted.
const MAX_API_RESPONSE: u64 = 4 << 20;

/// How long a fetched list is used before it is fetched again.
pub const REFRESH_AFTER_SECS: u64 = 24 * 60 * 60;

/// The releases known on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    /// Unix time of the last successful fetch (0: never).
    pub fetched_at: u64,
    /// Stable releases, newest first.
    pub releases: Vec<Release>,
}

impl Known {
    /// The newest stable release known.
    #[must_use]
    pub fn latest(&self) -> Option<&Release> {
        self.releases.first()
    }

    /// The release of version `version`, if known.
    #[must_use]
    pub fn find(&self, version: &str) -> Option<&Release> {
        self.releases.iter().find(|r| r.version == version)
    }

    /// Whether the list is older than [`REFRESH_AFTER_SECS`] at `now`.
    #[must_use]
    pub fn stale(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) >= REFRESH_AFTER_SECS
    }
}

fn known_path(cache: &Path) -> PathBuf {
    cache.join("releases.json")
}

/// The list as last saved (empty if never fetched or unreadable).
#[must_use]
pub fn load(cache: &Path) -> Known {
    std::fs::read_to_string(known_path(cache))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(cache: &Path, known: &Known) -> Result<()> {
    std::fs::create_dir_all(cache)?;
    let path = known_path(cache);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(known)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Fetch the release list from GitHub and save it.
///
/// `curl` with an argument vector, no `~/.curlrc`, HTTPS only, size-capped,
/// with a time limit — the same rules as a download.
///
/// # Errors
/// Returns an error if the request fails or lists no usable release; the
/// saved list is then left as it was.
pub fn refresh(cache: &Path) -> Result<Known> {
    let json = optiscaler::run(
        "curl",
        &[
            // First, or it is ignored: no ~/.curlrc may change what this does.
            "--disable".as_ref(),
            "--connect-timeout".as_ref(),
            "20".as_ref(),
            "--fail".as_ref(),
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--location".as_ref(),
            "--proto".as_ref(),
            "=https".as_ref(),
            "--proto-redir".as_ref(),
            "=https".as_ref(),
            "--max-time".as_ref(),
            "20".as_ref(),
            "--max-filesize".as_ref(),
            MAX_API_RESPONSE.to_string().as_ref(),
            "--header".as_ref(),
            "Accept: application/vnd.github+json".as_ref(),
            RELEASES_API.as_ref(),
        ],
    )?;
    let releases = optiscaler::parse_releases(&json)?;
    ensure!(
        !releases.is_empty(),
        "GitHub lists no stable OptiScaler release with a checksum"
    );
    let known = Known {
        fetched_at: now(),
        releases,
    };
    save(cache, &known)?;
    tracing::info!(target: "graphics", latest = %known.latest().map_or("", |r| r.version.as_str()),
        "OptiScaler release list refreshed");
    Ok(known)
}

/// The saved list, fetched again first when it is stale. A failed fetch
/// (offline, rate-limited) keeps the saved list: an update offer can wait.
#[must_use]
pub fn load_fresh(cache: &Path) -> Known {
    let known = load(cache);
    if !known.stale(now()) {
        return known;
    }
    match refresh(cache) {
        Ok(k) => k,
        Err(e) => {
            tracing::info!(target: "graphics", error = %e, "OptiScaler release list not refreshed");
            known
        }
    }
}

/// The release `policy` means, from what is known (no network).
///
/// # Errors
/// Returns an error if the policy names a release that is not known yet —
/// the latest before any list was fetched, or a pinned version that is
/// neither cached nor listed.
pub fn resolve(cache: &Path, policy: &VersionPolicy, known: &Known) -> Result<Release> {
    let recommended = Release::recommended();
    match policy {
        VersionPolicy::Recommended => Ok(recommended),
        VersionPolicy::Latest => match known.latest() {
            // Never older than what BiGame-mode was tested with.
            Some(l) if compare_versions(&l.version, &recommended.version).is_gt() => Ok(l.clone()),
            Some(_) => Ok(recommended),
            None => {
                bail!("the latest OptiScaler release is not known yet; check for updates first")
            }
        },
        VersionPolicy::Pinned(v) if *v == recommended.version => Ok(recommended),
        VersionPolicy::Pinned(v) => optiscaler::cached_version(cache, v)
            .map(|c| c.release)
            .or_else(|| known.find(v).cloned())
            .ok_or_else(|| {
                anyhow::anyhow!("OptiScaler {v} is not a stable release with a published checksum")
            }),
    }
}

/// The release a game's manifest says is installed — the same archive, by
/// its SHA-256 — found in the cache, the recommended release or the list.
///
/// # Errors
/// Returns an error if no known release matches, so that nothing is
/// "repaired" with a different version's files.
pub fn for_installed(cache: &Path, source: &Source) -> Result<Release> {
    let same = |r: &Release| {
        r.version == source.version
            && source
                .archive_sha256
                .as_deref()
                .is_none_or(|h| h.eq_ignore_ascii_case(&r.sha256))
    };
    let recommended = Release::recommended();
    if let Some(c) = optiscaler::cached_version(cache, &source.version) {
        if same(&c.release) {
            return Ok(c.release);
        }
    }
    if same(&recommended) {
        return Ok(recommended);
    }
    if let Some(r) = load(cache).releases.iter().find(|r| same(r)) {
        return Ok(r.clone());
    }
    bail!(
        "OptiScaler {} as installed is neither in the cache nor a known release",
        source.version
    )
}

/// A newer release to offer for a game that has `installed`.
///
/// Nothing is offered to a pinned game (the user chose to keep a version),
/// for a version the user skipped, or when nothing newer is known.
#[must_use]
pub fn offer(
    installed: &str,
    policy: &VersionPolicy,
    skipped: Option<&str>,
    known: &Known,
) -> Option<Release> {
    if matches!(policy, VersionPolicy::Pinned(_)) {
        return None;
    }
    let latest = known.latest()?;
    (compare_versions(&latest.version, installed).is_gt()
        && skipped != Some(latest.version.as_str()))
    .then(|| latest.clone())
}

/// What an update offer looks like for one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offer {
    /// The version installed now.
    pub installed: String,
    /// A newer release, when one is offered.
    pub available: Option<Release>,
    /// The version installed before the last update, which "Go back" returns
    /// to.
    pub previous: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(v: &str) -> Release {
        Release {
            tag: format!("v{v}"),
            version: v.into(),
            asset: format!("Optiscaler_{v}.7z"),
            sha256: format!("{:0>64}", v.replace('.', "")),
            size: 1,
            published: String::new(),
        }
    }

    fn known(vs: &[&str]) -> Known {
        Known {
            fetched_at: 100,
            releases: vs.iter().map(|v| rel(v)).collect(),
        }
    }

    #[test]
    fn versions_order_by_their_numbers_not_as_text() {
        use std::cmp::Ordering::{Greater, Less};
        assert_eq!(compare_versions("0.10.0", "0.9.4"), Greater);
        assert_eq!(compare_versions("0.9.4", "0.9.3"), Greater);
        assert_eq!(compare_versions("v0.9.2", "0.9.2a"), Less);
        assert!(compare_versions("0.9.4", "0.9.4").is_eq());
    }

    #[test]
    fn the_api_list_keeps_only_stable_releases_with_a_checksum_newest_first() {
        // Shaped like api.github.com/repos/optiscaler/OptiScaler/releases.
        let json = r#"[
          {"tag_name":"v0.9.3","draft":false,"prerelease":false,"published_at":"2026-06-18T00:00:00Z",
           "assets":[{"name":"Optiscaler_0.9.3-final.20260618.7z","size":53356866,"digest":"sha256:e3ac655d60ec11b471ac8cc5f4d3758e4bce9151c86caa339d8f0700c00282e3"}]},
          {"tag_name":"v0.9.4","draft":false,"prerelease":false,"published_at":"2026-07-18T00:00:00Z",
           "assets":[{"name":"Optiscaler_0.9.4-final.20260718._MM.7z","size":55016448,"digest":"sha256:575cb4df866116093df75af607e37fd70e10f5163e0f23fd5c804142e80ef0ad"}]},
          {"tag_name":"v0.7-old_nightly","draft":false,"prerelease":true,"published_at":"2025-07-05T00:00:00Z",
           "assets":[{"name":"a.7z","size":1,"digest":"sha256:69eca1ab4c65f69eca1ab4c65f69eca1ab4c65f69eca1ab4c65f69eca1ab4c65f69"}]},
          {"tag_name":"v0.9.1","draft":false,"prerelease":false,"published_at":"2026-04-27T00:00:00Z",
           "assets":[{"name":"Optiscaler_0.9.1.7z","size":1,"digest":null}]}
        ]"#;
        let list = optiscaler::parse_releases(json).unwrap();
        let versions: Vec<&str> = list.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(versions, ["0.9.4", "0.9.3"]);
        assert_eq!(list[0], Release::recommended());
        assert!(optiscaler::parse_releases("{}").is_err());
    }

    #[test]
    fn each_policy_resolves_to_one_release_and_unknown_ones_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let k = known(&["0.10.0", "0.9.4", "0.9.3"]);
        assert_eq!(
            resolve(dir.path(), &VersionPolicy::Recommended, &k).unwrap(),
            Release::recommended()
        );
        assert_eq!(
            resolve(dir.path(), &VersionPolicy::Latest, &k)
                .unwrap()
                .version,
            "0.10.0"
        );
        assert_eq!(
            resolve(dir.path(), &VersionPolicy::Pinned("0.9.3".into()), &k)
                .unwrap()
                .version,
            "0.9.3"
        );
        assert!(resolve(dir.path(), &VersionPolicy::Pinned("0.8.0".into()), &k).is_err());
        assert!(resolve(dir.path(), &VersionPolicy::Latest, &Known::default()).is_err());
        // "Latest" never goes below the tested release.
        assert_eq!(
            resolve(dir.path(), &VersionPolicy::Latest, &known(&["0.9.3"])).unwrap(),
            Release::recommended()
        );
    }

    #[test]
    fn an_installed_version_is_found_again_only_by_the_same_archive() {
        let dir = tempfile::tempdir().unwrap();
        let r = Release::recommended();
        let src = Source {
            component: "optiscaler".into(),
            backend: "optiscaler".into(),
            version: r.version.clone(),
            url: None,
            archive_sha256: Some(r.sha256.clone()),
        };
        assert_eq!(for_installed(dir.path(), &src).unwrap(), r);
        // Same version number, different archive (a re-upload): not the same.
        let other = Source {
            archive_sha256: Some("0".repeat(64)),
            ..src.clone()
        };
        assert!(for_installed(dir.path(), &other).is_err());
        // Known from a saved list.
        save(dir.path(), &known(&["0.9.3"])).unwrap();
        let old = Source {
            version: "0.9.3".into(),
            archive_sha256: Some(rel("0.9.3").sha256),
            ..src
        };
        assert_eq!(for_installed(dir.path(), &old).unwrap().version, "0.9.3");
    }

    #[test]
    fn a_newer_release_is_offered_unless_pinned_or_skipped() {
        let k = known(&["0.9.5", "0.9.4"]);
        let rec = VersionPolicy::Recommended;
        assert_eq!(offer("0.9.4", &rec, None, &k).unwrap().version, "0.9.5");
        assert!(offer("0.9.5", &rec, None, &k).is_none());
        assert!(offer("0.9.4", &rec, Some("0.9.5"), &k).is_none());
        assert!(offer("0.9.4", &VersionPolicy::Pinned("0.9.4".into()), None, &k).is_none());
        // A skipped version does not hide a newer one.
        let k2 = known(&["0.9.6", "0.9.5"]);
        assert_eq!(
            offer("0.9.4", &rec, Some("0.9.5"), &k2).unwrap().version,
            "0.9.6"
        );
        assert!(offer("0.9.4", &rec, None, &Known::default()).is_none());
    }

    #[test]
    fn the_list_is_saved_and_goes_stale_after_a_day() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Known::default());
        let k = known(&["0.9.4"]);
        save(dir.path(), &k).unwrap();
        assert_eq!(load(dir.path()), k);
        assert!(!k.stale(100 + REFRESH_AFTER_SECS - 1));
        assert!(k.stale(100 + REFRESH_AFTER_SECS));
    }
}
