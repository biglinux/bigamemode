//! What Steam knows about its apps: `appcache/appinfo.vdf`, a binary
//! `KeyValues` cache of every app's store data, read only for the installed
//! ones.
//!
//! Two things are taken from it: an app's `common/type` (`Game`, `Demo`,
//! `Tool`, …), which tells a game from a runtime, a dedicated server or an
//! SDK better than its title does; and its `config/launch` entries, the files
//! Steam starts (`bin\x64\Cyberpunk2077.exe`, `Nuts/Binaries/Win64/ItTakesTwo.exe`),
//! which name the game where file sizes only guess.
//!
//! Layout (versions 27, 28 and 29, as Steam writes them): a header of magic
//! and universe — version 29 adds the offset of a string table that holds
//! every key —, then one record per app (`appid`, `size`, fixed fields, the
//! app's binary `KeyValues`), ending with appid 0. The file is a cache Steam
//! rewrites; anything unexpected yields nothing rather than a guess.

use std::collections::{HashMap, HashSet};
use std::path::Path;

const MAGIC_27: u32 = 0x0756_4427;
const MAGIC_28: u32 = 0x0756_4428;
const MAGIC_29: u32 = 0x0756_4429;

/// Larger than any real cache (about 6 MB for a few hundred apps); a bigger
/// file is not read.
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// Nesting deeper than any real app record.
const MAX_DEPTH: u32 = 32;

/// One `config/launch` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Launch {
    /// `executable`, relative to the install directory, with Windows or Unix
    /// separators — or a URL (`link2ea://…`) for games another store starts.
    pub executable: String,
    /// `type`: `default`, `option1`, `none`, `vr`, … or absent.
    pub kind: Option<String>,
    /// `config/oslist`: `windows`, `linux`, `macos`, or several; absent means
    /// any.
    pub oslist: Option<String>,
    /// `config/BetaKey`: the branches the entry is for; absent means all.
    pub betakey: Option<String>,
}

/// What appinfo says about one app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct AppInfo {
    /// `common/type`, lowercased.
    pub kind: Option<String>,
    /// `config/launch`, in the order of their numeric keys.
    pub launch: Vec<Launch>,
}

/// The records of `wanted` in an `appinfo.vdf`; empty when the file is
/// missing, too large, or in a format this does not know.
pub(super) fn read(path: &Path, wanted: &HashSet<u32>) -> HashMap<u32, AppInfo> {
    if wanted.is_empty() {
        return HashMap::new();
    }
    let too_big = std::fs::metadata(path).map_or(true, |m| m.len() > MAX_FILE);
    if too_big {
        return HashMap::new();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    match parse(&bytes, wanted) {
        Ok(apps) => apps,
        Err(reason) => {
            tracing::debug!(file = %path.display(), reason, "Steam's app cache not read");
            HashMap::new()
        }
    }
}

/// A bounds-checked reader over the file.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], &'static str> {
        let end = self.pos.checked_add(n).ok_or("offset overflow")?;
        let slice = self.bytes.get(self.pos..end).ok_or("truncated")?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, &'static str> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// A NUL-terminated UTF-8 string.
    fn cstr(&mut self) -> Result<String, &'static str> {
        let rest = self.bytes.get(self.pos..).ok_or("truncated")?;
        let len = memchr::memchr(0, rest).ok_or("unterminated string")?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(s)
    }

    /// A NUL-terminated UTF-16 string, skipped.
    fn skip_wide(&mut self) -> Result<(), &'static str> {
        loop {
            if self.take(2)? == [0, 0] {
                return Ok(());
            }
        }
    }
}

/// A binary `KeyValues` value, as far as this needs: sections and strings.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Section(Vec<(String, Value)>),
    Text(String),
    Other,
}

impl Value {
    fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Section(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// How keys are stored: inline (27, 28) or as indexes into the string
/// table (29).
enum Keys<'a> {
    Inline,
    Table(Vec<&'a [u8]>),
}

fn parse(bytes: &[u8], wanted: &HashSet<u32>) -> Result<HashMap<u32, AppInfo>, &'static str> {
    let mut r = Reader { bytes, pos: 0 };
    let magic = r.u32()?;
    let _universe = r.u32()?;
    // Fixed fields between an app's size and its KeyValues: info state,
    // last update, PICS token, text SHA-1, change number — and from 28 on
    // the binary SHA-1.
    let (fixed, keys) = match magic {
        MAGIC_27 => (40, Keys::Inline),
        MAGIC_28 => (60, Keys::Inline),
        MAGIC_29 => {
            let offset = usize::try_from(r.u64()?).map_err(|_| "string table offset")?;
            (60, Keys::Table(string_table(bytes, offset)?))
        }
        _ => return Err("unknown appinfo version"),
    };
    let mut apps = HashMap::new();
    loop {
        let app_id = r.u32()?;
        if app_id == 0 {
            break;
        }
        let size = usize::try_from(r.u32()?).map_err(|_| "record size")?;
        let start = r.pos;
        let end = start.checked_add(size).ok_or("record size")?;
        if end > bytes.len() || size < fixed {
            return Err("truncated record");
        }
        if wanted.contains(&app_id) {
            let mut record = Reader {
                bytes: &bytes[..end],
                pos: start + fixed,
            };
            let root = section(&mut record, &keys, 0)?;
            apps.insert(app_id, app_info(&root));
            if apps.len() == wanted.len() {
                break;
            }
        }
        r.pos = end;
    }
    Ok(apps)
}

/// The strings of version 29's table: a count, then that many
/// NUL-terminated strings.
fn string_table(bytes: &[u8], offset: usize) -> Result<Vec<&[u8]>, &'static str> {
    let mut r = Reader { bytes, pos: offset };
    let count = r.u32()?;
    // Every string takes at least its terminator.
    if usize::try_from(count).map_or(true, |c| c > bytes.len().saturating_sub(r.pos)) {
        return Err("string table count");
    }
    let mut table = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let rest = bytes.get(r.pos..).ok_or("truncated string table")?;
        let len = memchr::memchr(0, rest).ok_or("unterminated string table")?;
        table.push(&rest[..len]);
        r.pos += len + 1;
    }
    Ok(table)
}

fn key(r: &mut Reader<'_>, keys: &Keys<'_>) -> Result<String, &'static str> {
    match keys {
        Keys::Inline => r.cstr(),
        Keys::Table(table) => {
            let index = usize::try_from(r.u32()?).map_err(|_| "key index")?;
            let raw = table.get(index).ok_or("key index out of range")?;
            Ok(String::from_utf8_lossy(raw).into_owned())
        }
    }
}

/// The entries of one section, up to its end marker.
fn section(r: &mut Reader<'_>, keys: &Keys<'_>, depth: u32) -> Result<Value, &'static str> {
    if depth > MAX_DEPTH {
        return Err("nested too deep");
    }
    let mut entries = Vec::new();
    loop {
        let kind = r.u8()?;
        if kind == 0x08 || kind == 0x0B {
            return Ok(Value::Section(entries));
        }
        let name = key(r, keys)?;
        let value = match kind {
            0x00 => section(r, keys, depth + 1)?,
            0x01 => Value::Text(r.cstr()?),
            // int32, float32, pointer, colour
            0x02 | 0x03 | 0x04 | 0x06 => {
                r.take(4)?;
                Value::Other
            }
            // uint64, int64
            0x07 | 0x0A => {
                r.take(8)?;
                Value::Other
            }
            0x05 => {
                r.skip_wide()?;
                Value::Other
            }
            _ => return Err("unknown value type"),
        };
        entries.push((name, value));
    }
}

fn app_info(root: &Value) -> AppInfo {
    // Records wrap everything in an `appinfo` section.
    let app = root.get("appinfo").unwrap_or(root);
    let kind = app
        .get("common")
        .and_then(|c| c.get("type"))
        .and_then(Value::text)
        .map(str::to_ascii_lowercase);
    let mut launch: Vec<(u32, Launch)> = Vec::new();
    if let Some(Value::Section(entries)) = app.get("config").and_then(|c| c.get("launch")) {
        for (index, entry) in entries {
            let Some(executable) = entry.get("executable").and_then(Value::text) else {
                continue;
            };
            let text = |v: Option<&Value>| v.and_then(Value::text).map(str::to_owned);
            let config = entry.get("config");
            launch.push((
                index.parse().unwrap_or(u32::MAX),
                Launch {
                    executable: executable.to_owned(),
                    kind: text(entry.get("type")).map(|t| t.to_ascii_lowercase()),
                    oslist: text(config.and_then(|c| c.get("oslist"))),
                    betakey: text(config.and_then(|c| c.get("betakey"))),
                },
            ));
        }
    }
    launch.sort_by_key(|(index, _)| *index);
    AppInfo {
        kind,
        launch: launch.into_iter().map(|(_, l)| l).collect(),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Writes appinfo files the way Steam lays them out.
    pub(in crate::games) struct Writer {
        version: u32,
        strings: Vec<String>,
        records: Vec<u8>,
    }

    /// A binary `KeyValues` value to write.
    pub(in crate::games) enum Kv {
        Section(Vec<(&'static str, Kv)>),
        Text(&'static str),
        Int(u32),
    }

    impl Writer {
        pub(in crate::games) fn new(version: u32) -> Self {
            Self {
                version,
                strings: Vec::new(),
                records: Vec::new(),
            }
        }

        fn key(&mut self, out: &mut Vec<u8>, key: &str) {
            if self.version == 29 {
                let index = self
                    .strings
                    .iter()
                    .position(|s| s == key)
                    .unwrap_or_else(|| {
                        self.strings.push(key.to_owned());
                        self.strings.len() - 1
                    });
                out.extend_from_slice(&u32::try_from(index).unwrap().to_le_bytes());
            } else {
                out.extend_from_slice(key.as_bytes());
                out.push(0);
            }
        }

        fn kv(&mut self, out: &mut Vec<u8>, entries: &[(&'static str, Kv)]) {
            for (key, value) in entries {
                match value {
                    Kv::Section(inner) => {
                        out.push(0x00);
                        self.key(out, key);
                        self.kv(out, inner);
                    }
                    Kv::Text(text) => {
                        out.push(0x01);
                        self.key(out, key);
                        out.extend_from_slice(text.as_bytes());
                        out.push(0);
                    }
                    Kv::Int(n) => {
                        out.push(0x02);
                        self.key(out, key);
                        out.extend_from_slice(&n.to_le_bytes());
                    }
                }
            }
            out.push(0x08);
        }

        pub(in crate::games) fn app(&mut self, app_id: u32, kv: &[(&'static str, Kv)]) {
            let mut body = vec![0u8; if self.version == 27 { 40 } else { 60 }];
            self.kv(&mut body, kv);
            self.records.extend_from_slice(&app_id.to_le_bytes());
            self.records
                .extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
            self.records.extend_from_slice(&body);
        }

        pub(in crate::games) fn finish(self) -> Vec<u8> {
            let magic = match self.version {
                27 => MAGIC_27,
                28 => MAGIC_28,
                _ => MAGIC_29,
            };
            let mut out = magic.to_le_bytes().to_vec();
            out.extend_from_slice(&1u32.to_le_bytes());
            let header = if self.version == 29 { 16 } else { 8 };
            let table_at = header + self.records.len() + 4;
            if self.version == 29 {
                out.extend_from_slice(&(table_at as u64).to_le_bytes());
            }
            out.extend_from_slice(&self.records);
            out.extend_from_slice(&0u32.to_le_bytes());
            if self.version == 29 {
                out.extend_from_slice(&u32::try_from(self.strings.len()).unwrap().to_le_bytes());
                for s in &self.strings {
                    out.extend_from_slice(s.as_bytes());
                    out.push(0);
                }
            }
            out
        }
    }

    /// Cyberpunk 2077's record as this machine's Steam has it: the default
    /// entry is the launcher, the game itself only on a beta branch.
    pub(in crate::games) fn cyberpunk() -> Vec<(&'static str, Kv)> {
        vec![(
            "appinfo",
            Kv::Section(vec![
                ("appid", Kv::Int(1_091_500)),
                (
                    "common",
                    Kv::Section(vec![
                        ("name", Kv::Text("Cyberpunk 2077")),
                        ("type", Kv::Text("Game")),
                    ]),
                ),
                (
                    "config",
                    Kv::Section(vec![(
                        "launch",
                        Kv::Section(vec![
                            (
                                "1",
                                Kv::Section(vec![
                                    ("executable", Kv::Text("redprelauncher.exe")),
                                    ("type", Kv::Text("default")),
                                    ("config", Kv::Section(vec![("oslist", Kv::Text("windows"))])),
                                ]),
                            ),
                            (
                                "2",
                                Kv::Section(vec![
                                    ("executable", Kv::Text("bin\\x64\\Cyberpunk2077.exe")),
                                    ("type", Kv::Text("default")),
                                    (
                                        "config",
                                        Kv::Section(vec![("BetaKey", Kv::Text("pc_preview_a"))]),
                                    ),
                                ]),
                            ),
                        ]),
                    )]),
                ),
            ]),
        )]
    }

    #[test]
    fn launch_entries_and_type_are_read_in_every_known_version() {
        for version in [27, 28, 29] {
            let mut w = Writer::new(version);
            w.app(
                10,
                &[(
                    "appinfo",
                    Kv::Section(vec![(
                        "common",
                        Kv::Section(vec![("type", Kv::Text("Tool"))]),
                    )]),
                )],
            );
            w.app(1_091_500, &cyberpunk());
            let bytes = w.finish();
            let apps = parse(&bytes, &HashSet::from([1_091_500, 10])).unwrap();
            assert_eq!(apps[&10].kind.as_deref(), Some("tool"), "v{version}");
            let cp = &apps[&1_091_500];
            assert_eq!(cp.kind.as_deref(), Some("game"));
            assert_eq!(cp.launch.len(), 2);
            assert_eq!(cp.launch[0].executable, "redprelauncher.exe");
            assert_eq!(cp.launch[0].oslist.as_deref(), Some("windows"));
            assert_eq!(cp.launch[1].betakey.as_deref(), Some("pc_preview_a"));
        }
    }

    #[test]
    fn an_unknown_or_damaged_file_yields_nothing() {
        let mut w = Writer::new(29);
        w.app(1_091_500, &cyberpunk());
        let mut bytes = w.finish();
        assert!(parse(&bytes[..bytes.len() / 2], &HashSet::from([1_091_500])).is_err());
        bytes[0] = 0x30;
        assert_eq!(
            parse(&bytes, &HashSet::from([1])),
            Err("unknown appinfo version")
        );
        assert!(parse(b"", &HashSet::from([1])).is_err());
    }

    #[test]
    fn this_machines_cache_reads_without_error() {
        let home = crate::paths::home_dir();
        let path = home.join(".local/share/Steam/appcache/appinfo.vdf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        assert!(parse(&bytes, &HashSet::from([u32::MAX - 1])).is_ok());
    }
}
