//! vkBasalt's look: its own CAS sharpening, or the Nara Linux style.
//!
//! vkBasalt reads `VKBASALT_CONFIG_FILE`, else
//! `~/.config/vkBasalt/vkBasalt.conf`. A style is written to that file; the
//! user's own file is kept aside first and comes back with [`Style::Own`],
//! as `MangoHud`'s styles do.
//!
//! The Nara Linux style — by Narayan, of the Nara Linux channel
//! (<https://www.youtube.com/watch?v=GGBC-qMB_0Y>) — uses three `ReShade`
//! shaders. vkBasalt aborts the game when an effect's file is missing, so
//! the style is written only after the shaders are in place: downloaded on
//! request from pinned commits, each checked against its SHA-256.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::error::UserError;
use crate::text::N_;

/// vkBasalt's example file, from its package.
pub const EXAMPLE: &str = "/usr/share/vkBasalt/vkBasalt.conf.example";

/// The Nara Linux channel's video with the style.
pub const NARA_VIDEO: &str = "https://www.youtube.com/watch?v=GGBC-qMB_0Y";

/// The first line of a file Big Game Mode wrote. Files already on users'
/// machines carry this spelling of the name, so it stays.
const MARKER: &str = "# Managed by BiGame-mode";

/// Narayan's file, with `/home/USERNAME/.local/share/reshade` for the
/// shader folder.
const NARA_TEMPLATE: &str = include_str!("../../../data/vkBasalt.conf");

/// The folder the template names.
const TEMPLATE_DIR: &str = "/home/USERNAME/.local/share/reshade";

/// A look for vkBasalt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Whatever the user's file says (or vkBasalt's defaults).
    Own,
    /// vkBasalt's built-in Contrast Adaptive Sharpening.
    Cas,
    /// Colourfulness, `FakeHDR` and `FilmGrain2`, from `ReShade`'s shaders.
    NaraLinux,
}

/// What `vkBasalt.conf` holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleState {
    /// No file.
    Defaults,
    /// A file Big Game Mode did not write.
    Own,
    /// A style Big Game Mode wrote.
    Style(Style),
}

/// One shader file: where it goes under the shader folder, and where it
/// comes from.
struct Shader {
    dest: &'static str,
    repo: &'static str,
    commit: &'static str,
    path: &'static str,
    sha256: &'static str,
}

/// `ReShade`'s legacy shaders (Colourfulness, `FilmGrain2`).
const LEGACY: &str = "bcb5ba54199f4455026dd8ba66dc1b74461d3152";
/// `ReShade`'s slim branch (the headers every shader includes).
const SLIM: &str = "fd0022170615ce0d8162d219bff07232fa6dd84f";
/// `SweetFX` (`FakeHDR`).
const SWEETFX: &str = "93ddf39b357f5da534ed6d34ba4ec8cc7dcfa361";

const SHADERS: &[Shader] = &[
    Shader {
        dest: "shaders/Colourfulness.fx",
        repo: "crosire/reshade-shaders",
        commit: LEGACY,
        path: "Shaders/Colourfulness.fx",
        sha256: "0add0a1e22ff62f1bee8b61d6bbc49d2e162dedf055bb6d167425706a4725c04",
    },
    Shader {
        dest: "shaders/FilmGrain2.fx",
        repo: "crosire/reshade-shaders",
        commit: LEGACY,
        path: "Shaders/FilmGrain2.fx",
        sha256: "d52f8f4326e6bec2ea605a2ce54a702f2bedd27c46fea61da87e7b766abfd20a",
    },
    Shader {
        dest: "shaders/ReShade.fxh",
        repo: "crosire/reshade-shaders",
        commit: SLIM,
        path: "Shaders/ReShade.fxh",
        sha256: "6dabfbbaf968c3871905d2ea17f96572ff7b1cec01310b5d0e5252b66b30174f",
    },
    Shader {
        dest: "shaders/ReShadeUI.fxh",
        repo: "crosire/reshade-shaders",
        commit: SLIM,
        path: "Shaders/ReShadeUI.fxh",
        sha256: "78adf672df47460297eb9fe6dd238d2aafa24510b52b84feb1a745dff70eb901",
    },
    Shader {
        dest: "shaders/SweetFX/FakeHDR.fx",
        repo: "CeeJayDK/SweetFX",
        commit: SWEETFX,
        path: "Shaders/SweetFX/FakeHDR.fx",
        sha256: "f6ca5ed2c4f0698695cca7fc0a2b917e1934b6cb34064a0bc7aa098051727cf9",
    },
    Shader {
        dest: "shaders/SweetFX/LICENSE",
        repo: "CeeJayDK/SweetFX",
        commit: SWEETFX,
        path: "LICENSE",
        sha256: "9b2e0b3ff53493f211e39390947716cd16ee559303dac0713ad4b12228dbce51",
    },
];

/// No shader file is this large; a bigger answer is not one.
const MAX_SHADER: u64 = 256 * 1024;

/// vkBasalt's own file.
#[must_use]
pub fn user_config() -> PathBuf {
    crate::paths::config_home().join("vkBasalt/vkBasalt.conf")
}

/// Where the user's own file is kept while a style is in place.
fn backup_path() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/vkbasalt/vkBasalt.conf.user")
}

/// Where the Nara Linux style's shaders go.
#[must_use]
pub fn shader_dir() -> PathBuf {
    crate::paths::data_home().join("reshade")
}

/// The file vkBasalt reads when no path is set: the user's own, else the
/// package's example.
#[must_use]
pub fn default_config() -> Option<PathBuf> {
    let own = user_config();
    if own.is_file() {
        return Some(own);
    }
    let example = PathBuf::from(EXAMPLE);
    example.is_file().then_some(example)
}

/// Which style is in place.
#[must_use]
pub fn current_style() -> StyleState {
    state_of(std::fs::read_to_string(user_config()).ok().as_deref())
}

fn state_of(text: Option<&str>) -> StyleState {
    let Some(text) = text else {
        return StyleState::Defaults;
    };
    let first = text.lines().next().unwrap_or_default();
    match first.strip_prefix(MARKER) {
        Some(rest) if rest.contains("style nara-linux") => StyleState::Style(Style::NaraLinux),
        Some(rest) if rest.contains("style cas") => StyleState::Style(Style::Cas),
        _ => StyleState::Own,
    }
}

/// Whether every shader the Nara Linux style uses is in `dir`, unchanged.
#[must_use]
pub fn shaders_ready_in(dir: &Path) -> bool {
    SHADERS.iter().all(|s| {
        crate::graphics::manifest::sha256_file(&dir.join(s.dest)).is_ok_and(|h| h == s.sha256)
    })
}

/// [`shaders_ready_in`] the shader folder.
#[must_use]
pub fn shaders_ready() -> bool {
    shaders_ready_in(&shader_dir())
}

/// The file for `style`, with `dir` as the shader folder; `None` for
/// [`Style::Own`].
#[must_use]
pub fn style_config(style: Style, dir: &Path) -> Option<String> {
    match style {
        Style::Own => None,
        Style::Cas => Some(format!(
            "{MARKER}: style cas.\n\
             # Contrast Adaptive Sharpening, built into vkBasalt. Home switches it on and off.\n\
             effects = cas\n\
             casSharpness = 0.4\n\
             toggleKey = Home\n\
             enableOnLaunch = True\n"
        )),
        Style::NaraLinux => Some(format!(
            "{MARKER}: style nara-linux.\n\
             # Nara Linux style, by Narayan (Nara Linux channel): {NARA_VIDEO}\n\
             # Shaders from ReShade (crosire/reshade-shaders) and SweetFX (CeeJayDK/SweetFX).\n\
             {}",
            NARA_TEMPLATE.replace(TEMPLATE_DIR, &dir.to_string_lossy())
        )),
    }
}

/// Download the Nara Linux style's shaders into `dir`, each checked before
/// it is put in place. Files already there and unchanged are kept.
///
/// `curl` gets an argument vector: HTTPS only, redirects only to HTTPS,
/// failing on HTTP errors and on more than `MAX_SHADER` bytes.
///
/// # Errors
/// Returns an error if a download fails or does not match its hash.
pub fn fetch_shaders(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("textures"))?;
    for s in SHADERS {
        let dest = dir.join(s.dest);
        if crate::graphics::manifest::sha256_file(&dest).is_ok_and(|h| h == s.sha256) {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let part = dest.with_extension("part");
        let _ = std::fs::remove_file(&part);
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}",
            s.repo, s.commit, s.path
        );
        tracing::info!(target: "vkbasalt", %url, "downloading a shader");
        let out = std::process::Command::new("curl")
            .args([
                // First, or it is ignored: no ~/.curlrc may change what this does.
                "--disable",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-filesize",
                &MAX_SHADER.to_string(),
                "--connect-timeout",
                "20",
                "--max-time",
                "120",
                "--output",
            ])
            .arg(&part)
            .arg(&url)
            .output()
            .context("run curl")?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&part);
            bail!(UserError::with(
                N_("could not download %s: %s"),
                [
                    s.path.to_owned(),
                    String::from_utf8_lossy(&out.stderr).trim().to_owned()
                ]
            ));
        }
        let got = crate::graphics::manifest::sha256_file(&part)?;
        if got != s.sha256 {
            let _ = std::fs::remove_file(&part);
            bail!(UserError::with(
                N_("download does not match %s: SHA-256 %s"),
                [s.path.to_owned(), got]
            ));
        }
        std::fs::rename(&part, &dest).with_context(|| format!("put {}", dest.display()))?;
    }
    ensure!(shaders_ready_in(dir), "the shaders are not all in place");
    tracing::info!(target: "vkbasalt", dir = %dir.display(), "Nara Linux shaders in place and verified");
    Ok(())
}

/// Put `style` in place. The first time, the user's own file (if any) is
/// kept aside; [`Style::Own`] puts it back, or removes Big Game Mode's file
/// when there was none. The Nara Linux style needs its shaders
/// ([`fetch_shaders`]) first.
///
/// # Errors
/// Returns an error if the shaders are missing or a file cannot be moved or
/// written.
pub fn set_style(style: Style) -> Result<()> {
    set_style_at(style, &user_config(), &backup_path(), &shader_dir())
}

fn set_style_at(style: Style, path: &Path, backup: &Path, dir: &Path) -> Result<()> {
    let state = state_of(std::fs::read_to_string(path).ok().as_deref());
    match style_config(style, dir) {
        None => {
            if !matches!(state, StyleState::Style(_)) {
                return Ok(()); // Already the user's own.
            }
            if backup.exists() {
                std::fs::rename(backup, path)
                    .with_context(|| format!("put back {}", path.display()))?;
            } else {
                std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
        Some(text) => {
            // A missing effect file makes vkBasalt abort the game.
            if style == Style::NaraLinux && !shaders_ready_in(dir) {
                bail!(UserError::plain(N_(
                    "the Nara Linux style's shaders are not downloaded"
                )));
            }
            if state == StyleState::Own {
                if let Some(parent) = backup.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(path, backup).with_context(|| format!("keep {}", path.display()))?;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("conf.bigame-new");
            std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
            std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_style_is_recognised_by_its_first_line() {
        let dir = Path::new("/x/reshade");
        assert_eq!(state_of(None), StyleState::Defaults);
        assert_eq!(state_of(Some("effects = cas\n")), StyleState::Own);
        for style in [Style::Cas, Style::NaraLinux] {
            let text = style_config(style, dir).unwrap();
            assert_eq!(state_of(Some(&text)), StyleState::Style(style));
        }
    }

    #[test]
    fn the_nara_style_names_the_shader_folder_and_its_effects() {
        let text = style_config(
            Style::NaraLinux,
            Path::new("/home/ana/.local/share/reshade"),
        )
        .unwrap();
        assert!(!text.contains("USERNAME"));
        assert!(
            text.contains(
                "Colourfulness = /home/ana/.local/share/reshade/shaders/Colourfulness.fx"
            )
        );
        assert!(
            text.contains("FakeHDR = /home/ana/.local/share/reshade/shaders/SweetFX/FakeHDR.fx")
        );
        let effects = text
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("effects = "))
            .unwrap();
        // Every effect the style turns on is a file this module downloads.
        for effect in effects.split(':') {
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{effect} = ")))
                .unwrap();
            let rel = line
                .strip_prefix("/home/ana/.local/share/reshade/")
                .unwrap();
            assert!(SHADERS.iter().any(|s| s.dest == rel), "{effect}: {rel}");
        }
    }

    #[test]
    fn the_nara_style_waits_for_its_shaders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vkBasalt/vkBasalt.conf");
        let backup = dir.path().join("state/vkBasalt.conf.user");
        let shaders = dir.path().join("reshade");
        assert!(set_style_at(Style::NaraLinux, &path, &backup, &shaders).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn the_users_own_file_is_kept_and_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vkBasalt/vkBasalt.conf");
        let backup = dir.path().join("state/vkBasalt.conf.user");
        let shaders = dir.path().join("reshade");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "effects = smaa\n").unwrap();

        set_style_at(Style::Cas, &path, &backup, &shaders).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().starts_with(MARKER));
        // A second style does not overwrite the kept file with ours.
        set_style_at(Style::Cas, &path, &backup, &shaders).unwrap();
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "effects = smaa\n"
        );

        set_style_at(Style::Own, &path, &backup, &shaders).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "effects = smaa\n");
        assert!(!backup.exists());
    }

    #[test]
    fn without_a_file_of_its_own_ours_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vkBasalt/vkBasalt.conf");
        let backup = dir.path().join("state/vkBasalt.conf.user");
        let shaders = dir.path().join("reshade");
        set_style_at(Style::Cas, &path, &backup, &shaders).unwrap();
        assert!(path.exists());
        set_style_at(Style::Own, &path, &backup, &shaders).unwrap();
        assert!(!path.exists());
    }
}
