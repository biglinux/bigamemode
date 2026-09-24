//! `OptiScaler`: getting a release, configuring it for one game, and reading
//! back whether it is working.
//!
//! `OptiScaler` (github.com/optiscaler/OptiScaler, GPL-3.0) sits between a
//! game and its upscaler: it takes the upscaler the game already supports
//! (DLSS, `XeSS` or FSR — the *input*) and runs another in its place (the
//! *output*), and can add frame generation. It is loaded by the game as a
//! proxy DLL — normally `dxgi.dll`, which Proton already loads natively from
//! the game folder — with its settings in `OptiScaler.ini` beside it.
//!
//! BiGame-mode never ships it. A release is downloaded from the project's own
//! GitHub releases when the user asks for it, checked against a known SHA-256
//! (or the digest GitHub publishes for a newer release), unpacked into
//! BiGame-mode's cache once, and copied from there into each game through
//! [`super::transaction`], which backs up whatever it replaces.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::manifest::{self, FileKind, check_relative, sha256_file};
use super::scan::{GameScan, ProxyOwner};
use super::transaction::PlannedFile;

/// Component id used in manifests and the cache.
pub const COMPONENT: &str = "optiscaler";

/// Largest archive accepted.
const MAX_ARCHIVE: u64 = 300 << 20;
/// Largest total size an archive may unpack to.
const MAX_UNPACKED: u64 = 1 << 30;

/// One `OptiScaler` release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Git tag (`v0.9.4`).
    pub tag: String,
    /// Version (`0.9.4`).
    pub version: String,
    /// Release asset file name.
    pub asset: String,
    /// SHA-256 of the asset.
    pub sha256: String,
    /// Size of the asset in bytes.
    pub size: u64,
    /// Publication date (`YYYY-MM-DD`).
    pub published: String,
}

impl Release {
    /// The release BiGame-mode was tested with — the version a profile gets
    /// unless the user picks another. Hash and size as published by GitHub
    /// for the asset (`sha256:` digest), checked against a download.
    #[must_use]
    pub fn recommended() -> Self {
        Self {
            tag: "v0.9.4".into(),
            version: "0.9.4".into(),
            asset: "Optiscaler_0.9.4-final.20260718._MM.7z".into(),
            sha256: "575cb4df866116093df75af607e37fd70e10f5163e0f23fd5c804142e80ef0ad".into(),
            size: 55_016_448,
            published: "2026-07-18".into(),
        }
    }

    /// Download URL on the project's GitHub releases.
    #[must_use]
    pub fn url(&self) -> String {
        format!(
            "https://github.com/optiscaler/OptiScaler/releases/download/{}/{}",
            self.tag, self.asset
        )
    }
}

/// Read the latest stable release from the GitHub API response for
/// `repos/optiscaler/OptiScaler/releases/latest`.
///
/// The asset is found by extension, not by name (its suffix changes between
/// releases), and must be the only `.7z`. Only a release that GitHub marks
/// as neither a draft nor a pre-release, and whose asset carries a SHA-256
/// digest, is accepted — without a published digest there is nothing to
/// check a download against.
///
/// # Errors
/// Returns an error if the response is not a usable stable release.
pub fn parse_latest(json: &str) -> Result<Release> {
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        size: u64,
        digest: Option<String>,
    }
    #[derive(Deserialize)]
    struct Api {
        tag_name: String,
        draft: bool,
        prerelease: bool,
        published_at: Option<String>,
        assets: Vec<Asset>,
    }
    let api: Api = serde_json::from_str(json).context("GitHub release JSON")?;
    ensure!(
        !api.draft && !api.prerelease,
        "{} is not a stable release",
        api.tag_name
    );
    let archives: Vec<&Asset> = api
        .assets
        .iter()
        .filter(|a| {
            Path::new(&a.name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("7z"))
        })
        .collect();
    ensure!(
        archives.len() == 1,
        "expected one .7z asset in {}, found {}",
        api.tag_name,
        archives.len()
    );
    let a = archives[0];
    let sha = a
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .with_context(|| format!("{} has no SHA-256 digest to check against", a.name))?;
    ensure!(
        a.name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "unexpected asset name {:?}",
        a.name
    );
    Ok(Release {
        version: api.tag_name.trim_start_matches('v').to_owned(),
        tag: api.tag_name,
        asset: a.name.clone(),
        sha256: sha.to_ascii_lowercase(),
        size: a.size,
        published: api
            .published_at
            .unwrap_or_default()
            .chars()
            .take(10)
            .collect(),
    })
}

/// BiGame-mode's cache for `OptiScaler` releases, shared by every game.
#[must_use]
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join(".cache")
        })
        .join("bigame-mode/graphics/optiscaler")
}

/// A release that is in the cache, checked and unpacked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cached {
    /// The release.
    pub release: Release,
    /// Where it was unpacked.
    pub dir: PathBuf,
    /// Where it was downloaded from.
    pub url: String,
    /// Unix time of the download.
    pub downloaded_at: u64,
    /// License of `OptiScaler` itself; bundled components carry their own
    /// licenses in the release's `Licenses/` folder.
    pub license: String,
}

impl Cached {
    /// The manifest source for installs from this release.
    #[must_use]
    pub fn source(&self) -> manifest::Source {
        manifest::Source {
            component: COMPONENT.into(),
            version: self.release.version.clone(),
            url: Some(self.url.clone()),
            archive_sha256: Some(self.release.sha256.clone()),
        }
    }
}

fn record_path(cache: &Path, release: &Release) -> PathBuf {
    cache.join(&release.version).join("release.json")
}

/// The cached copy of `release`, if it is there and is the same release.
#[must_use]
pub fn cached(cache: &Path, release: &Release) -> Option<Cached> {
    let text = std::fs::read_to_string(record_path(cache, release)).ok()?;
    let c: Cached = serde_json::from_str(&text).ok()?;
    (c.release.sha256 == release.sha256 && c.dir.join("OptiScaler.dll").is_file()).then_some(c)
}

/// Check an archive listing before anything is extracted.
///
/// `verbose` is `bsdtar -tvf` output (for entry types), `names` is
/// `bsdtar -tf` output (one name per line, as stored). Both list the same
/// entries in the same order. Only regular files and directories with plain
/// relative names are accepted: no absolute paths, no `..`, no symlinks,
/// hard links or devices.
///
/// # Errors
/// Returns an error naming the first entry that is not acceptable.
pub fn check_listing(verbose: &str, names: &str) -> Result<Vec<PathBuf>> {
    let kinds: Vec<char> = verbose.lines().filter_map(|l| l.chars().next()).collect();
    let names: Vec<&str> = names.lines().collect();
    ensure!(
        kinds.len() == names.len() && !names.is_empty(),
        "archive listing is inconsistent ({} types, {} names)",
        kinds.len(),
        names.len()
    );
    let mut out = Vec::with_capacity(names.len());
    for (kind, name) in kinds.into_iter().zip(names) {
        let rel = Path::new(name.trim_end_matches('/'));
        check_relative(rel).with_context(|| format!("archive entry {name:?}"))?;
        match kind {
            '-' | 'd' => out.push(rel.to_path_buf()),
            other => bail!("archive entry {name:?} is not a plain file or folder (type {other:?})"),
        }
    }
    Ok(out)
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;
    ensure!(
        out.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Unpack `archive` into `dest` (which must not exist), checking its listing
/// first and what was written after.
///
/// `bsdtar` (libarchive, which pacman itself depends on) reads the 7-Zip
/// format `OptiScaler` is released in. It is given argument vectors, never a
/// shell line, and told not to restore owners or permissions.
///
/// # Errors
/// Returns an error — and removes anything partly written — if the listing
/// or the result is not acceptable or `bsdtar` fails.
pub fn extract_safely(archive: &Path, dest: &Path) -> Result<()> {
    ensure!(!dest.exists(), "{} already exists", dest.display());
    let verbose = run("bsdtar", &["-tvf".as_ref(), archive.as_os_str()])?;
    let names = run("bsdtar", &["-tf".as_ref(), archive.as_os_str()])?;
    check_listing(&verbose, &names)?;
    let tmp = dest.with_extension("unpacking");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let result = (|| -> Result<()> {
        run(
            "bsdtar",
            &[
                "-xf".as_ref(),
                archive.as_os_str(),
                "-C".as_ref(),
                tmp.as_os_str(),
                "--no-same-owner".as_ref(),
                "--no-same-permissions".as_ref(),
            ],
        )?;
        // What was written, not only what was listed.
        let mut total = 0u64;
        let mut stack = vec![tmp.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir)?.flatten() {
                let ft = e.file_type()?;
                if ft.is_dir() {
                    stack.push(e.path());
                } else if ft.is_file() {
                    total += e.metadata()?.len();
                } else {
                    bail!("unpacked {} is not a plain file", e.path().display());
                }
            }
        }
        ensure!(total <= MAX_UNPACKED, "archive unpacks to {total} bytes");
        std::fs::rename(&tmp, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result
}

/// Download `release` into the cache, check it, and unpack it — or return
/// the copy already there.
///
/// `curl` (which pacman also depends on) is run with an argument vector:
/// HTTPS only, following redirects only to HTTPS (GitHub serves assets from
/// its CDN), failing on HTTP errors, and refusing more than the expected
/// size. The file is hashed before anything else looks at it.
///
/// # Errors
/// Returns an error if the download fails or does not match the release.
pub fn fetch(cache: &Path, release: &Release) -> Result<Cached> {
    if let Some(c) = cached(cache, release) {
        return Ok(c);
    }
    ensure!(
        release.size <= MAX_ARCHIVE,
        "release asset is {} bytes",
        release.size
    );
    let dir = cache.join(&release.version);
    std::fs::create_dir_all(&dir)?;
    let part = dir.join("download.part");
    let _ = std::fs::remove_file(&part);
    let url = release.url();
    tracing::info!(target: "graphics", %url, "downloading OptiScaler");
    run(
        "curl",
        &[
            "--fail".as_ref(),
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--location".as_ref(),
            "--proto".as_ref(),
            "=https".as_ref(),
            "--proto-redir".as_ref(),
            "=https".as_ref(),
            "--max-filesize".as_ref(),
            release.size.to_string().as_ref(),
            "--output".as_ref(),
            part.as_os_str(),
            url.as_ref(),
        ],
    )?;
    let got = sha256_file(&part)?;
    if got != release.sha256 {
        let _ = std::fs::remove_file(&part);
        bail!("download does not match {}: SHA-256 {got}", release.tag);
    }
    let archive = dir.join(&release.asset);
    std::fs::rename(&part, &archive)?;
    let unpacked = dir.join("files");
    let _ = std::fs::remove_dir_all(&unpacked);
    extract_safely(&archive, &unpacked)?;
    let c = Cached {
        release: release.clone(),
        dir: unpacked,
        url,
        downloaded_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        license: "GPL-3.0".into(),
    };
    std::fs::write(
        record_path(cache, release),
        serde_json::to_string_pretty(&c)?,
    )?;
    tracing::info!(target: "graphics", version = %release.version, "OptiScaler downloaded and verified");
    Ok(c)
}

/// Set `key` in `[section]` of an ini text, keeping everything else —
/// comments, order, the user's other settings — as it was.
///
/// The key is replaced where it is (the first uncommented occurrence in that
/// section), added at the end of the section if absent, and the section is
/// appended if missing. Keys and sections compare case-insensitively, as
/// `OptiScaler`'s ini reader does.
#[must_use]
pub fn set_ini(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = |l: &str| {
        let t = l.trim();
        (t.starts_with('[') && t.ends_with(']')).then(|| t[1..t.len() - 1].trim().to_owned())
    };
    let start = lines
        .iter()
        .position(|l| header(l).is_some_and(|h| h.eq_ignore_ascii_case(section)));
    let new_line = format!("{key}={value}");
    match start {
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(new_line);
        }
        Some(s) => {
            let end = lines[s + 1..]
                .iter()
                .position(|l| header(l).is_some())
                .map_or(lines.len(), |i| s + 1 + i);
            let existing = (s + 1..end).find(|&i| {
                let t = lines[i].trim_start();
                !t.starts_with(';')
                    && !t.starts_with('#')
                    && t.split_once('=')
                        .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key))
            });
            if let Some(i) = existing {
                lines[i] = new_line;
            } else {
                // After the last non-blank line of the section.
                let at = (s + 1..end)
                    .rev()
                    .find(|&i| !lines[i].trim().is_empty())
                    .map_or(s + 1, |i| i + 1);
                lines.insert(at, new_line);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The graphics API the game renders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Api {
    /// Direct3D 11.
    Dx11,
    /// Direct3D 12.
    Dx12,
    /// Vulkan.
    Vulkan,
}

/// Which of the game's own upscalers `OptiScaler` takes over — the one to
/// select in the game's menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// The game's DLSS. On AMD and Intel the game hides it unless it is told
    /// the GPU is NVIDIA, so this enables `OptiScaler`'s spoofing.
    Dlss,
    /// The game's `XeSS`.
    Xess,
    /// The game's FSR 2/3.
    Fsr,
}

/// What `OptiScaler` runs in place of the game's upscaler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    /// AMD FSR through `OptiScaler`'s FSR 3.1 backend, which runs FSR 4 on
    /// RDNA 3 and 4 (`Fsr4Update`) and FSR 3.1 elsewhere.
    Fsr,
    /// Intel `XeSS` (the release's `libxess`).
    Xess,
    /// NVIDIA DLSS — NVIDIA GPUs only, with the game's own DLSS runtime.
    Dlss,
}

/// Frame generation through `OptiScaler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameGen {
    /// None.
    Off,
    /// `OptiFG`: generated from the upscaler's inputs, output by FSR frame
    /// generation.
    OptiFgFsr,
}

/// How `OptiScaler` is set up for one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    /// DLL slot it loads through (`dxgi.dll`).
    pub proxy: String,
    /// The game's API.
    pub api: Api,
    /// The game's upscaler to take over.
    pub input: Input,
    /// What to run instead.
    pub output: Output,
    /// Frame generation.
    pub frame_gen: FrameGen,
    /// Whether the GPU is NVIDIA (spoofing is never needed then).
    pub nvidia: bool,
    /// Show `OptiScaler`'s FSR 4 watermark, which says whether FSR 4 really
    /// runs or fell back to FSR 3 — for validation.
    pub watermark: bool,
}

/// The ini settings for `o`, as `(section, key, value)`.
///
/// Everything not listed stays at the release's `auto`. Two are always set:
/// the log goes to a file at info level — that file is how BiGame-mode knows
/// `OptiScaler` really loaded and what it runs — and `OptiScaler` does not
/// check the internet for updates from inside the game: updates are
/// BiGame-mode's, and the user's decision.
#[must_use]
pub fn ini_settings(o: &Options) -> Vec<(&'static str, &'static str, String)> {
    let mut s = vec![
        ("Log", "LogToFile", "true".to_owned()),
        ("Log", "LogLevel", "2".to_owned()),
        ("Hotfix", "CheckForUpdate", "false".to_owned()),
    ];
    let backend = match o.output {
        Output::Fsr => match o.api {
            Api::Dx12 => "fsr31",
            // Through D3D12 interop, which is what reaches FSR 4.
            Api::Dx11 | Api::Vulkan => "fsr31_12",
        },
        Output::Xess => match o.api {
            Api::Dx11 => "xess_12",
            Api::Dx12 | Api::Vulkan => "xess",
        },
        Output::Dlss => "dlss",
    };
    let key = match o.api {
        Api::Dx11 => "Dx11Upscaler",
        Api::Dx12 => "Dx12Upscaler",
        Api::Vulkan => "VulkanUpscaler",
    };
    s.push(("Upscalers", key, backend.to_owned()));
    // DLSS inputs are hidden by the game on AMD/Intel unless the GPU is
    // reported as NVIDIA. For XeSS or FSR inputs, spoofing only risks sending
    // the game down NVIDIA code paths, so it is turned off.
    let spoof = o.input == Input::Dlss && !o.nvidia;
    s.push(("Spoofing", "Dxgi", spoof.to_string()));
    if o.watermark && o.output == Output::Fsr {
        s.push(("FSR", "Fsr4EnableWatermark", "true".to_owned()));
    }
    match o.frame_gen {
        FrameGen::Off => s.push(("FrameGen", "Enabled", "false".to_owned())),
        FrameGen::OptiFgFsr => {
            s.push(("FrameGen", "Enabled", "true".to_owned()));
            s.push(("FrameGen", "FGInput", "upscaler".to_owned()));
            s.push(("FrameGen", "FGOutput", "fsrfg".to_owned()));
        }
    }
    s
}

/// Release files `o` needs, beside `OptiScaler.dll` and its ini.
///
/// The smallest set that works: the game's own upscaler DLLs are left alone
/// unless the output needs a newer one (`OptiScaler` hooks whichever
/// `libxess.dll` the game has loaded, so `XeSS` *input* needs none), and
/// the `D3D12_Optiscaler` Agility SDK copy — licensed by Microsoft for
/// Windows only, and of no use under VKD3D-Proton, which implements D3D12 —
/// is never copied.
#[must_use]
pub fn release_files(o: &Options) -> Vec<&'static str> {
    let mut f = Vec::new();
    match (o.output, o.api) {
        (Output::Fsr, Api::Vulkan) => f.push("amd_fidelityfx_vk.dll"),
        (Output::Fsr, _) => {
            f.push("amd_fidelityfx_dx12.dll");
            f.push("amd_fidelityfx_upscaler_dx12.dll");
        }
        (Output::Xess, Api::Dx11) => f.extend(["libxess.dll", "libxess_dx11.dll"]),
        (Output::Xess, _) => f.push("libxess.dll"),
        (Output::Dlss, _) => {}
    }
    if o.frame_gen == FrameGen::OptiFgFsr && o.api != Api::Vulkan {
        f.push("amd_fidelityfx_dx12.dll");
        f.push("amd_fidelityfx_framegeneration_dx12.dll");
    }
    if o.input == Input::Dlss && !o.nvidia {
        f.extend(["fakenvapi.dll", "fakenvapi.ini"]);
    }
    f.sort_unstable();
    f.dedup();
    f
}

/// Why `OptiScaler` cannot go into a slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotTaken {
    /// The slot.
    pub slot: String,
    /// Who has it.
    pub owner: ProxyOwner,
}

/// Pick the DLL slot for `OptiScaler` in a scanned game.
///
/// `dxgi.dll` is the one upstream recommends and the one Proton already loads
/// natively from the game folder (it sets `dxgi` to native for DXVK), so it
/// needs no override. If another tool already has it, that is reported
/// rather than overwritten: which of two DXGI hooks should win is the user's
/// decision, and chaining them is `OptiScaler`'s own feature to configure.
///
/// # Errors
/// Returns the slot and its owner when the slot is taken by something else.
pub fn choose_slot(scan: &GameScan) -> Result<String, SlotTaken> {
    let slot = "dxgi.dll";
    match scan.proxies.iter().find(|p| p.slot == slot) {
        None => Ok(slot.to_owned()),
        Some(p) if p.owner == ProxyOwner::OptiScaler => Ok(slot.to_owned()),
        Some(p) => Err(SlotTaken {
            slot: slot.to_owned(),
            owner: p.owner.clone(),
        }),
    }
}

/// The files to place for `o` from a cached release: `OptiScaler.dll` under
/// its slot name, a configured `OptiScaler.ini` (written to `staging`), and
/// [`release_files`]. Paths are relative to the install folder, in
/// `exe_dir_rel` (where the game's executable is).
///
/// # Errors
/// Returns an error if the release lacks a needed file or the ini cannot be
/// written.
pub fn payload(
    cached: &Cached,
    o: &Options,
    exe_dir_rel: &Path,
    staging: &Path,
) -> Result<Vec<PlannedFile>> {
    check_relative(&exe_dir_rel.join(&o.proxy))?;
    let mut files = vec![PlannedFile {
        path: exe_dir_rel.join(&o.proxy),
        source: cached.dir.join("OptiScaler.dll"),
        kind: FileKind::Binary,
    }];
    let base = std::fs::read_to_string(cached.dir.join("OptiScaler.ini"))
        .context("the release's OptiScaler.ini")?;
    let ini = ini_settings(o)
        .into_iter()
        .fold(base, |text, (sec, key, val)| set_ini(&text, sec, key, &val));
    std::fs::create_dir_all(staging)?;
    let ini_path = staging.join("OptiScaler.ini");
    std::fs::write(&ini_path, ini)?;
    files.push(PlannedFile {
        path: exe_dir_rel.join("OptiScaler.ini"),
        source: ini_path,
        kind: FileKind::Config,
    });
    for name in release_files(o) {
        let source = cached.dir.join(name);
        ensure!(source.is_file(), "the release has no {name}");
        let kind = if Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ini"))
        {
            FileKind::Config
        } else {
            FileKind::Binary
        };
        files.push(PlannedFile {
            path: exe_dir_rel.join(name),
            source,
            kind,
        });
    }
    Ok(files)
}

/// What `OptiScaler`'s log says about one run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogFindings {
    /// `OptiScaler v… loaded`.
    pub version: Option<String>,
    /// The slot it said it loaded as (`dxgi.dll`).
    pub working_as: Option<String>,
    /// `Running on Wine …`.
    pub wine: bool,
    /// Upscaler backends it created (`fsr31`, `xess`, …), in order.
    pub upscalers: Vec<String>,
    /// The FSR 4 line: `RDNA4: true, RDNA3: false, Fsr4Update: true`.
    pub fsr4: Option<String>,
    /// Whether AMD's FSR 4 runtime (`amdxcffx64.dll`) was loaded: `Some(true)`
    /// for `amdxcffx64 loaded from …`, `Some(false)` for `Failed to load
    /// amdxcffx64.dll` — after which `OptiScaler` goes on with FSR 3.1. `None`
    /// when it did not try.
    pub amdxcffx64: Option<bool>,
    /// Lines that say something failed.
    pub errors: Vec<String>,
}

impl LogFindings {
    /// The backend in use now: the last one created.
    #[must_use]
    pub fn current_upscaler(&self) -> Option<&str> {
        self.upscalers.last().map(String::as_str)
    }

    /// Which FSR the `fsr31` backend really runs: 4 or 3 (3.1), from the log,
    /// or `None` when the log does not settle it. FSR 4 needs both
    /// `Fsr4Update: true` and AMD's runtime loaded (`FSR4Upgrade.cpp`); without
    /// either, the backend is FSR 3.1.
    #[must_use]
    pub fn fsr_generation(&self) -> Option<u8> {
        if !self.current_upscaler()?.starts_with("fsr31") {
            return None;
        }
        if self
            .fsr4
            .as_deref()
            .is_some_and(|l| l.contains("Fsr4Update: false"))
        {
            return Some(3);
        }
        self.amdxcffx64.map(|loaded| if loaded { 4 } else { 3 })
    }
}

/// Read `OptiScaler.log`.
///
/// Lines look like `[HH:MM:SS.ffffff] [I] <function> <message>`. Matching is
/// on the messages `OptiScaler` v0.9.4 writes (`dllmain.cpp`,
/// `NVNGX_DLSS_Dx12.cpp`, `FSR4Upgrade.cpp`), not on the functions.
#[must_use]
pub fn read_log(text: &str) -> LogFindings {
    let mut f = LogFindings::default();
    for line in text.lines() {
        if let Some(rest) = line.split("OptiScaler v").nth(1) {
            if line.contains(" loaded") && f.version.is_none() {
                f.version = rest.split_whitespace().next().map(str::to_owned);
            }
        }
        if line.contains("Running on Wine") {
            f.wine = true;
        }
        if let Some(rest) = line.split("OptiScaler working as ").nth(1) {
            f.working_as = rest
                .split(|c: char| c == ',' || c.is_whitespace())
                .next()
                .map(str::to_owned);
        }
        if let Some(rest) = line.split("Creating new ").nth(1) {
            if let Some(name) = rest
                .strip_suffix(" upscaler")
                .or_else(|| rest.split_once(" upscaler").map(|(n, _)| n))
            {
                f.upscalers.push(name.trim().to_owned());
            }
        }
        if line.contains("Fsr4Update:") {
            f.fsr4 = line.find("RDNA4:").map(|i| line[i..].trim().to_owned());
        }
        if line.contains("amdxcffx64 loaded") {
            f.amdxcffx64 = Some(true);
        }
        // A warning, not a failure: OptiScaler goes on with FSR 3.1. Under
        // Proton this is the usual case — the DLL comes with AMD's Windows
        // driver.
        let fsr4_runtime_missing = line.contains("Failed to load amdxcffx64");
        if fsr4_runtime_missing {
            f.amdxcffx64 = Some(false);
        }
        let failed = !fsr4_runtime_missing
            && (line.contains("can't load")
                || line.contains("Upscaler can't created")
                || line.contains("Failed to load")
                || line.contains("] [E] "));
        if failed {
            let msg = line
                .splitn(3, "] ")
                .nth(2)
                .unwrap_or(line)
                .trim()
                .to_owned();
            if !f.errors.contains(&msg) {
                f.errors.push(msg);
            }
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::scan::Proxy;

    fn opts(input: Input, output: Output, api: Api) -> Options {
        Options {
            proxy: "dxgi.dll".into(),
            api,
            input,
            output,
            frame_gen: FrameGen::Off,
            nvidia: false,
            watermark: false,
        }
    }

    #[test]
    fn ini_keys_are_set_in_their_section_and_nothing_else_moves() {
        let text = "; header\n[Upscalers]\n; Dx12Upscaler=auto (comment)\nDx12Upscaler=auto\nDx11Upscaler=auto\n\n[Log]\nLogLevel=auto\n";
        let out = set_ini(text, "Upscalers", "Dx12Upscaler", "fsr31");
        assert!(
            out.contains("; Dx12Upscaler=auto (comment)\nDx12Upscaler=fsr31\n"),
            "{out}"
        );
        let out = set_ini(&out, "log", "LogToFile", "true");
        assert!(
            out.contains("[Log]\nLogLevel=auto\nLogToFile=true\n"),
            "{out}"
        );
        let out = set_ini(&out, "Hotfix", "CheckForUpdate", "false");
        assert!(out.ends_with("\n[Hotfix]\nCheckForUpdate=false\n"), "{out}");
        assert!(out.starts_with("; header\n"));
        // The same key in another section is not touched.
        let two = set_ini("[A]\nx=1\n[B]\nx=1\n", "B", "x", "2");
        assert_eq!(two, "[A]\nx=1\n[B]\nx=2\n");
    }

    #[test]
    fn sottr_on_rdna4_gets_fsr_from_its_xess_with_no_spoofing_and_its_own_xess_kept() {
        let o = opts(Input::Xess, Output::Fsr, Api::Dx12);
        let s = ini_settings(&o);
        assert!(s.contains(&("Upscalers", "Dx12Upscaler", "fsr31".into())));
        assert!(s.contains(&("Spoofing", "Dxgi", "false".into())));
        assert!(s.contains(&("Hotfix", "CheckForUpdate", "false".into())));
        assert!(s.contains(&("Log", "LogToFile", "true".into())));
        let files = release_files(&o);
        assert_eq!(
            files,
            [
                "amd_fidelityfx_dx12.dll",
                "amd_fidelityfx_upscaler_dx12.dll"
            ]
        );
        assert!(
            !files.contains(&"libxess.dll"),
            "the game's own XeSS is the input"
        );
    }

    #[test]
    fn dlss_input_on_amd_brings_spoofing_and_fakenvapi_and_never_the_agility_sdk() {
        let o = opts(Input::Dlss, Output::Fsr, Api::Dx12);
        assert!(ini_settings(&o).contains(&("Spoofing", "Dxgi", "true".into())));
        let files = release_files(&o);
        assert!(files.contains(&"fakenvapi.dll") && files.contains(&"fakenvapi.ini"));
        assert!(!files.iter().any(|f| f.contains("D3D12")));
        let mut nv = o;
        nv.nvidia = true;
        assert!(ini_settings(&nv).contains(&("Spoofing", "Dxgi", "false".into())));
        assert!(!release_files(&nv).contains(&"fakenvapi.dll"));
    }

    #[test]
    fn dx11_and_vulkan_reach_fsr_through_interop() {
        let s = ini_settings(&opts(Input::Fsr, Output::Fsr, Api::Dx11));
        assert!(s.contains(&("Upscalers", "Dx11Upscaler", "fsr31_12".into())));
        let v = opts(Input::Fsr, Output::Fsr, Api::Vulkan);
        assert!(ini_settings(&v).contains(&("Upscalers", "VulkanUpscaler", "fsr31_12".into())));
        assert_eq!(release_files(&v), ["amd_fidelityfx_vk.dll"]);
    }

    #[test]
    fn frame_generation_adds_its_runtime_and_ini_keys() {
        let mut o = opts(Input::Xess, Output::Fsr, Api::Dx12);
        o.frame_gen = FrameGen::OptiFgFsr;
        let s = ini_settings(&o);
        assert!(s.contains(&("FrameGen", "FGInput", "upscaler".into())));
        assert!(s.contains(&("FrameGen", "FGOutput", "fsrfg".into())));
        assert!(release_files(&o).contains(&"amd_fidelityfx_framegeneration_dx12.dll"));
    }

    #[test]
    fn a_dxgi_slot_owned_by_another_tool_is_reported_not_taken() {
        let mut scan = GameScan::default();
        assert_eq!(choose_slot(&scan).unwrap(), "dxgi.dll");
        scan.proxies.push(Proxy {
            slot: "dxgi.dll".into(),
            path: "dxgi.dll".into(),
            owner: ProxyOwner::ReShade,
            version: None,
        });
        assert_eq!(
            choose_slot(&scan),
            Err(SlotTaken {
                slot: "dxgi.dll".into(),
                owner: ProxyOwner::ReShade
            })
        );
        scan.proxies[0].owner = ProxyOwner::OptiScaler;
        assert_eq!(choose_slot(&scan).unwrap(), "dxgi.dll");
    }

    #[test]
    fn listings_with_escapes_links_or_devices_are_refused() {
        let ok_v = "drwxr-xr-x 0 0 0 0 Jul 16 D3D12_Optiscaler/\n-rw-r--r-- 0 0 0 400 Mar 2 !! README !!.txt\n-rw-r--r-- 0 0 0 9 Jul 18 OptiScaler.dll\n";
        let ok_n = "D3D12_Optiscaler/\n!! README !!.txt\nOptiScaler.dll\n";
        assert_eq!(check_listing(ok_v, ok_n).unwrap().len(), 3);
        assert!(check_listing("-rw 1\n", "../../.bashrc\n").is_err());
        assert!(check_listing("-rw 1\n", "/etc/passwd\n").is_err());
        assert!(check_listing("lrwxrwxrwx 1\n", "dxgi.dll\n").is_err());
        assert!(check_listing("hrw 1\n", "dxgi.dll\n").is_err());
        assert!(check_listing("crw 1\n", "null\n").is_err());
        assert!(
            check_listing("-rw 1\n-rw 1\n", "a\n").is_err(),
            "inconsistent listing"
        );
    }

    #[test]
    fn a_real_archive_is_unpacked_only_when_its_listing_is_clean() {
        if std::process::Command::new("bsdtar")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // bsdtar missing: nothing to test against
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("Licenses")).unwrap();
        std::fs::write(src.join("OptiScaler.dll"), b"MZ").unwrap();
        std::fs::write(src.join("Licenses/x.txt"), b"x").unwrap();
        let good = dir.path().join("good.zip");
        let st = std::process::Command::new("bsdtar")
            .args(["--format", "zip", "-cf"])
            .arg(&good)
            .arg("-C")
            .arg(&src)
            .args(["OptiScaler.dll", "Licenses"])
            .status()
            .unwrap();
        assert!(st.success());
        let out = dir.path().join("out");
        extract_safely(&good, &out).unwrap();
        assert!(out.join("Licenses/x.txt").is_file());

        // A symlink in the archive stops the extraction before it starts.
        std::os::unix::fs::symlink("/etc/passwd", src.join("dxgi.dll")).unwrap();
        let bad = dir.path().join("bad.zip");
        let st = std::process::Command::new("bsdtar")
            .args(["--format", "zip", "-cf"])
            .arg(&bad)
            .arg("-C")
            .arg(&src)
            .args(["OptiScaler.dll", "dxgi.dll"])
            .status()
            .unwrap();
        assert!(st.success());
        let out2 = dir.path().join("out2");
        assert!(extract_safely(&bad, &out2).is_err());
        assert!(!out2.exists() && !out2.with_extension("unpacking").exists());
    }

    #[test]
    fn the_log_says_what_loaded_what_runs_and_what_failed() {
        let log = "\
[10:00:00.000001] [W] OptiScaler v0.9.4-final (6ded74b) loaded
[10:00:00.000002] [I] CheckWingMode Running on Wine 10.0!
[10:00:00.000003] [I] CheckWorkingMode OptiScaler working as dxgi.dll, system dll loaded
[10:00:00.000004] [I] FSR4Upgrade RDNA4: true, RDNA3: false, Fsr4Update: true
[10:00:01.000000] [I] NVSDK_NGX_D3D12_CreateFeature Creating new fsr31 upscaler
[10:00:02.000000] [E] FSR31FeatureDx12 can't load amd_fidelityfx_dx12.dll methods!
";
        let f = read_log(log);
        assert_eq!(f.version.as_deref(), Some("0.9.4-final"));
        assert!(f.wine);
        assert_eq!(f.working_as.as_deref(), Some("dxgi.dll"));
        assert_eq!(f.current_upscaler(), Some("fsr31"));
        assert_eq!(
            f.fsr4.as_deref(),
            Some("RDNA4: true, RDNA3: false, Fsr4Update: true")
        );
        assert_eq!(f.errors.len(), 1);
        assert!(f.errors[0].contains("amd_fidelityfx_dx12.dll"));
        assert_eq!(read_log(""), LogFindings::default());
    }

    #[test]
    fn fsr4_is_claimed_only_when_the_log_says_amds_runtime_loaded() {
        // Messages as FSR4Upgrade.cpp (v0.9.4) writes them.
        let head = "\
[1] [I] FSR4Upgrade RDNA4: true, RDNA3: false, Fsr4Update: true
[2] [I] NVSDK_NGX_D3D12_CreateFeature Creating new fsr31 upscaler
";
        let loaded = format!("{head}[3] [I] UpdateFfxApiProvider amdxcffx64 loaded from game folder\n");
        let f = read_log(&loaded);
        assert_eq!(f.fsr_generation(), Some(4));
        assert!(f.errors.is_empty());

        // The usual case under Proton: no AMD Windows driver, so no
        // amdxcffx64.dll. OptiScaler warns and runs FSR 3.1 — not a failure.
        let missing = format!("{head}[3] [W] UpdateFfxApiProvider Failed to load amdxcffx64.dll\n");
        let f = read_log(&missing);
        assert_eq!(f.fsr_generation(), Some(3));
        assert!(f.errors.is_empty(), "{:?}", f.errors);

        // Not an RDNA 3/4 GPU: FSR 4 is off from the start.
        let off = "[1] [I] FSR4Upgrade RDNA4: false, RDNA3: false, Fsr4Update: false\n[2] [I] f Creating new fsr31 upscaler\n";
        assert_eq!(read_log(off).fsr_generation(), Some(3));

        // FSR 4 on, but nothing said about the runtime yet: not settled.
        assert_eq!(read_log(head).fsr_generation(), None);
        // Not an FSR backend at all.
        assert_eq!(
            read_log("[1] [I] f Creating new xess upscaler\n").fsr_generation(),
            None
        );
    }

    #[test]
    fn only_a_stable_release_with_one_archive_and_a_digest_is_taken_from_the_api() {
        let json = r#"{"tag_name":"v0.9.5","draft":false,"prerelease":false,"published_at":"2026-10-01T10:00:00Z",
            "assets":[{"name":"Optiscaler_0.9.5-final.7z","size":100,"digest":"sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789"}]}"#;
        let r = parse_latest(json).unwrap();
        assert_eq!(
            (r.tag.as_str(), r.version.as_str(), r.published.as_str()),
            ("v0.9.5", "0.9.5", "2026-10-01")
        );
        assert_eq!(
            r.sha256,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert!(
            parse_latest(&json.replace(r#""prerelease":false"#, r#""prerelease":true"#)).is_err()
        );
        assert!(parse_latest(&json.replace("sha256:ABCDEF", "md5:ABCDEF")).is_err());
        assert!(parse_latest(&json.replace(".7z", ".7z/../../x")).is_err());
        let two = json.replace("}]}", r#"},{"name":"b.7z","size":1,"digest":null}]}"#);
        assert!(parse_latest(&two).is_err());
        assert_eq!(
            Release::recommended().url(),
            "https://github.com/optiscaler/OptiScaler/releases/download/v0.9.4/Optiscaler_0.9.4-final.20260718._MM.7z"
        );
    }
}
