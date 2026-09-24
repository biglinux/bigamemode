//! Just enough of the PE/COFF format to answer three questions about a
//! Windows binary: which CPU it is for, which DLLs it links against, and
//! whether a given marker string appears in it.
//!
//! Everything read comes from files in a game's folder, so every offset is
//! bounds-checked and every loop is capped: a truncated or hostile file yields
//! an error or a partial answer, never a panic or a runaway scan.
//!
//! Layout references: Microsoft's "PE Format" specification — DOS header
//! `e_lfanew` at 0x3C, COFF header after the `PE\0\0` signature, optional
//! header magic 0x10B (PE32) / 0x20B (PE32+), data directory 1 (imports) and
//! 13 (delay-load imports), 40-byte section headers.

use std::io::Read;
use std::path::Path;

/// The CPU a binary is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Machine {
    /// 32-bit x86 (`IMAGE_FILE_MACHINE_I386`).
    X86,
    /// 64-bit x86 (`IMAGE_FILE_MACHINE_AMD64`).
    X64,
    /// 64-bit ARM (`IMAGE_FILE_MACHINE_ARM64`).
    Arm64,
    /// Anything else.
    Other,
}

/// What was read from a PE file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeInfo {
    /// Target CPU, if the header was readable.
    pub machine: Option<Machine>,
    /// DLLs named in the import table, lowercased, in file order.
    pub imports: Vec<String>,
    /// DLLs named in the delay-load import table, lowercased.
    pub delay_imports: Vec<String>,
}

impl PeInfo {
    /// Whether `dll` (case-insensitive) is imported directly or delay-loaded.
    #[must_use]
    pub fn links(&self, dll: &str) -> bool {
        let dll = dll.to_ascii_lowercase();
        self.imports
            .iter()
            .chain(&self.delay_imports)
            .any(|d| *d == dll)
    }
}

/// Why a file could not be read as PE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeError {
    /// No `MZ` header.
    NotPe,
    /// A header or table points outside the file.
    Truncated,
    /// An optional-header magic other than PE32 or PE32+.
    UnknownFormat,
}

impl std::fmt::Display for PeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotPe => "not a PE file",
            Self::Truncated => "truncated PE file",
            Self::UnknownFormat => "unknown PE optional header",
        })
    }
}

impl std::error::Error for PeError {}

const MAX_DESCRIPTORS: usize = 4096;
const MAX_NAME: usize = 256;

fn u16_at(b: &[u8], off: usize) -> Result<u16, PeError> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(PeError::Truncated)
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, PeError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(PeError::Truncated)
}

struct Section {
    va: u32,
    size: u32,
    raw: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Option<usize> {
    sections
        .iter()
        .find(|s| rva >= s.va && rva - s.va < s.size)
        .map(|s| (rva - s.va + s.raw) as usize)
}

fn c_string(b: &[u8], off: usize) -> Option<String> {
    let tail = b.get(off..)?;
    let end = tail.iter().take(MAX_NAME).position(|&c| c == 0)?;
    let s = std::str::from_utf8(&tail[..end]).ok()?;
    (!s.is_empty() && s.is_ascii()).then(|| s.to_ascii_lowercase())
}

struct Headers {
    machine: Machine,
    sections: Vec<Section>,
    dirs_off: usize,
    dirs_count: usize,
}

fn headers(b: &[u8]) -> Result<Headers, PeError> {
    if b.get(0..2) != Some(b"MZ") {
        return Err(PeError::NotPe);
    }
    let pe = u32_at(b, 0x3C)? as usize;
    if b.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(PeError::NotPe);
    }
    let coff = pe + 4;
    let machine = match u16_at(b, coff)? {
        0x014c => Machine::X86,
        0x8664 => Machine::X64,
        0xaa64 => Machine::Arm64,
        _ => Machine::Other,
    };
    let sections_n = usize::from(u16_at(b, coff + 2)?);
    let opt_size = usize::from(u16_at(b, coff + 16)?);
    let opt = coff + 20;
    let (dirs_count_off, dirs_off) = match u16_at(b, opt)? {
        0x10b => (opt + 92, opt + 96),
        0x20b => (opt + 108, opt + 112),
        _ => return Err(PeError::UnknownFormat),
    };
    let dirs_count = u32_at(b, dirs_count_off)? as usize;

    let mut sections = Vec::with_capacity(sections_n.min(96));
    let table = opt + opt_size;
    for i in 0..sections_n.min(96) {
        let s = table + i * 40;
        let vsize = u32_at(b, s + 8)?;
        let raw_size = u32_at(b, s + 16)?;
        sections.push(Section {
            va: u32_at(b, s + 12)?,
            size: vsize.max(raw_size),
            raw: u32_at(b, s + 20)?,
        });
    }
    Ok(Headers {
        machine,
        sections,
        dirs_off,
        dirs_count,
    })
}

/// Parse the headers and import tables of a PE image held in memory.
///
/// # Errors
/// Returns [`PeError`] when the bytes are not a readable PE image. Import
/// entries that point outside the file are skipped rather than failing the
/// whole parse.
pub fn parse(b: &[u8]) -> Result<PeInfo, PeError> {
    parse_with(b, &|off, len| {
        b.get(off..)
            .map(|tail| tail[..len.min(tail.len())].to_vec())
    })
}

/// Parse from the first bytes of an image (`head`, which must hold the
/// headers and section table) plus a positioned reader for the rest.
///
/// Import tables of large executables are deep in the file — Cyberpunk
/// 2077's 740-byte table sits 53 MB into a 60 MB executable — so a file is
/// read in small pieces at the offsets the headers give, not as a whole.
fn parse_with(
    head: &[u8],
    read: &dyn Fn(usize, usize) -> Option<Vec<u8>>,
) -> Result<PeInfo, PeError> {
    let Headers {
        machine,
        sections,
        dirs_off,
        dirs_count,
    } = headers(head)?;

    let dir = |index: usize| -> Option<(u32, u32)> {
        (index < dirs_count)
            .then(|| {
                Some((
                    u32_at(head, dirs_off + index * 8).ok()?,
                    u32_at(head, dirs_off + index * 8 + 4).ok()?,
                ))
            })
            .flatten()
            .filter(|&(rva, _)| rva != 0)
    };
    let name_at = |rva: u32| -> Option<String> {
        let off = rva_to_offset(&sections, rva)?;
        c_string(&read(off, MAX_NAME + 1)?, 0)
    };
    // A descriptor table's declared size bounds how much of it is read; some
    // linkers under-report it, so at least a generous minimum is read.
    let table = |rva: u32, size: u32, entry: usize| -> Option<Vec<u8>> {
        let off = rva_to_offset(&sections, rva)?;
        let len = (size as usize).max(entry * 64).min(entry * MAX_DESCRIPTORS);
        read(off, len)
    };

    let mut imports = Vec::new();
    if let Some(t) = dir(1).and_then(|(rva, size)| table(rva, size, 20)) {
        for desc in t.chunks_exact(20).take(MAX_DESCRIPTORS) {
            if desc.iter().all(|&x| x == 0) {
                break;
            }
            if let Some(name) = name_at(u32_at(desc, 12)?) {
                imports.push(name);
            }
        }
    }

    let mut delay_imports = Vec::new();
    if let Some(t) = dir(13).and_then(|(rva, size)| table(rva, size, 32)) {
        for desc in t.chunks_exact(32).take(MAX_DESCRIPTORS) {
            let (attributes, name_rva) = (u32_at(desc, 0)?, u32_at(desc, 4)?);
            if name_rva == 0 {
                break;
            }
            // Attribute bit 0 clear is the pre-VC7 layout, where fields are
            // virtual addresses rather than RVAs; too old to matter for games
            // that have a modern upscaler, and not worth guessing an image base.
            if attributes & 1 == 0 {
                continue;
            }
            if let Some(name) = name_at(name_rva) {
                delay_imports.push(name);
            }
        }
    }

    Ok(PeInfo {
        machine: Some(machine),
        imports,
        delay_imports,
    })
}

/// Read at most `limit` bytes of a file.
///
/// # Errors
/// Returns the I/O error if the file cannot be opened or read.
pub fn read_prefix(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Parse a PE file on disk.
///
/// Only the headers, the descriptor tables and the names they point to are
/// read, at the offsets the headers give — a few kilobytes whatever the size
/// of the file. Nothing at or beyond `limit` bytes is read.
///
/// # Errors
/// Returns an error if the file cannot be read or is not a readable PE.
pub fn parse_file(path: &Path, limit: u64) -> anyhow::Result<PeInfo> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::open(path)?;
    let head = read_prefix(path, 64 * 1024)?;
    let read = |off: usize, len: usize| -> Option<Vec<u8>> {
        if off as u64 >= limit {
            return None;
        }
        let mut buf = vec![0u8; len];
        let n = file.read_at(&mut buf, off as u64).ok()?;
        buf.truncate(n);
        Some(buf)
    };
    Ok(parse_with(&head, &read)?)
}

/// The file version of a PE file on disk, read through its resource
/// directory.
///
/// The resource directory is a three-level tree (type, name, language) at the
/// start of `.rsrc`; type 16 is `RT_VERSION`, and its leaf gives the RVA and
/// size of the version block. Following it reads a few hundred bytes wherever
/// the block is — Intel's 77 MB `XeSS` runtime keeps it past its first 64 MB.
/// Files whose tree cannot be followed fall back to searching the first
/// megabyte of the section.
#[must_use]
pub fn read_file_version(path: &Path) -> Option<String> {
    read_version_block(path).and_then(|b| file_version(&b))
}

/// The raw version resource of a PE file on disk: `VS_VERSIONINFO` with its
/// string tables (`CompanyName`, `ProductName`, `FileDescription`, …), found
/// as [`read_file_version`] finds it. A few hundred bytes that say who made
/// the file, which is what telling a proxy DLL's owner needs first.
#[must_use]
pub fn read_version_block(path: &Path) -> Option<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::open(path).ok()?;
    let read = |off: usize, len: usize| -> Option<Vec<u8>> {
        let mut buf = vec![0u8; len];
        let n = file.read_at(&mut buf, off as u64).ok()?;
        buf.truncate(n);
        Some(buf)
    };
    let head = read(0, 64 * 1024)?;
    let h = headers(&head).ok()?;
    if h.dirs_count <= 2 {
        return None;
    }
    let rsrc_rva = u32_at(&head, h.dirs_off + 16).ok()?;
    let rsrc_size = u32_at(&head, h.dirs_off + 20).ok()?;
    let rsrc_off = rva_to_offset(&h.sections, rsrc_rva)?;
    let tree = read(rsrc_off, (rsrc_size as usize).min(64 * 1024))?;

    let leaf = version_leaf(&tree).and_then(|(rva, size)| {
        let off = rva_to_offset(&h.sections, rva)?;
        read(off, (size as usize).min(64 * 1024))
    });
    match leaf {
        Some(block) if file_version(&block).is_some() => Some(block),
        _ => {
            let span = read(rsrc_off, (rsrc_size as usize).min(1 << 20))?;
            file_version(&span).is_some().then_some(span)
        }
    }
}

/// Follow a resource tree (`tree` = the start of `.rsrc`) to the first
/// `RT_VERSION` leaf, returning its data RVA and size.
fn version_leaf(tree: &[u8]) -> Option<(u32, u32)> {
    const RT_VERSION: u32 = 16;
    const SUBDIR: u32 = 0x8000_0000;
    // Entries of the directory at `off`: (name-or-id, offset-to-data).
    let entries = |off: usize| -> Option<Vec<(u32, u32)>> {
        let named = usize::from(u16_at(tree, off + 12).ok()?);
        let ids = usize::from(u16_at(tree, off + 14).ok()?);
        (0..(named + ids).min(256))
            .map(|i| {
                let e = off + 16 + i * 8;
                Some((u32_at(tree, e).ok()?, u32_at(tree, e + 4).ok()?))
            })
            .collect()
    };
    let (_, types) = entries(0)?
        .into_iter()
        .find(|&(id, data)| id == RT_VERSION && data & SUBDIR != 0)?;
    let (_, names) = *entries((types & !SUBDIR) as usize)?.first()?;
    if names & SUBDIR == 0 {
        return None;
    }
    let (_, lang) = *entries((names & !SUBDIR) as usize)?.first()?;
    if lang & SUBDIR != 0 {
        return None;
    }
    let data = lang as usize;
    Some((u32_at(tree, data).ok()?, u32_at(tree, data + 4).ok()?))
}

/// Whether `marker` appears in `bytes` as ASCII or as UTF-16LE text.
///
/// UTF-16LE covers Windows version resources (`ProductName`, `CompanyName`),
/// which is where most DLLs say what they are.
#[must_use]
pub fn contains_marker(bytes: &[u8], marker: &str) -> bool {
    let ascii = marker.as_bytes();
    if !ascii.is_empty() && bytes.windows(ascii.len()).any(|w| w == ascii) {
        return true;
    }
    let wide: Vec<u8> = marker.encode_utf16().flat_map(u16::to_le_bytes).collect();
    !wide.is_empty() && bytes.windows(wide.len()).any(|w| w == wide.as_slice())
}

/// The file version recorded in a Windows binary's version resource.
///
/// The resource is a `VS_VERSIONINFO` block: three WORDs, the UTF-16 key
/// `VS_VERSION_INFO` with its terminator, padding to a DWORD boundary, then
/// `VS_FIXEDFILEINFO` — signature `0xFEEF04BD`, structure version,
/// `dwFileVersionMS`, `dwFileVersionLS`. Searching for the signature alone is
/// not enough: the four bytes turn up by chance in large DLLs (NVIDIA's DLSS
/// runtime has one before its real version block), so the search is anchored
/// on the key.
#[must_use]
pub fn file_version(bytes: &[u8]) -> Option<String> {
    const SIGNATURE: [u8; 4] = 0xFEEF_04BD_u32.to_le_bytes();
    let key: Vec<u8> = "VS_VERSION_INFO\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut from = 0;
    while let Some(pos) = bytes
        .get(from..)
        .and_then(|tail| tail.windows(key.len()).position(|w| w == key.as_slice()))
    {
        let key_end = from + pos + key.len();
        // At most 3 bytes of padding separate the key from the structure.
        if let Some(at) = (key_end..key_end + 4).find(|&i| bytes.get(i..i + 4) == Some(&SIGNATURE))
        {
            let ms = u32_at(bytes, at + 8).ok()?;
            let ls = u32_at(bytes, at + 12).ok()?;
            if ms != 0 || ls != 0 {
                return Some(format!(
                    "{}.{}.{}.{}",
                    ms >> 16,
                    ms & 0xFFFF,
                    ls >> 16,
                    ls & 0xFFFF
                ));
            }
        }
        from = key_end;
    }
    None
}

#[cfg(test)]
// Test images are a few hundred bytes; no offset in them can truncate.
#[allow(clippy::cast_possible_truncation)]
pub(crate) mod fixture {
    //! A tiny PE32+ image builder, so the parser is tested against bytes whose
    //! every field is known, not against whatever DLLs a machine happens to have.

    /// Build a PE image importing `imports` and delay-loading `delay`, with
    /// `extra` appended to the single section's data.
    pub fn pe(
        machine: u16,
        pe32plus: bool,
        imports: &[&str],
        delay: &[&str],
        extra: &[u8],
    ) -> Vec<u8> {
        let pe_off = 0x80usize;
        let opt_size: usize = if pe32plus { 240 } else { 224 };
        let table = pe_off + 24 + opt_size;
        let raw = 0x400usize;
        let va = 0x1000u32;

        // Section payload: import descriptors, then delay descriptors, then names.
        let mut data = Vec::new();
        let imp_desc = 0usize;
        let imp_len = (imports.len() + 1) * 20;
        let delay_desc = imp_desc + imp_len;
        let delay_len = (delay.len() + 1) * 32;
        let names = delay_desc + delay_len;
        data.resize(names, 0);
        let mut name_rvas = Vec::new();
        for n in imports.iter().chain(delay) {
            name_rvas.push(va + data.len() as u32);
            data.extend_from_slice(n.as_bytes());
            data.push(0);
        }
        for (i, rva) in name_rvas.iter().take(imports.len()).enumerate() {
            let d = imp_desc + i * 20;
            data[d..d + 4].copy_from_slice(&1u32.to_le_bytes()); // non-zero thunk
            data[d + 12..d + 16].copy_from_slice(&rva.to_le_bytes());
        }
        for (i, rva) in name_rvas.iter().skip(imports.len()).enumerate() {
            let d = delay_desc + i * 32;
            data[d..d + 4].copy_from_slice(&1u32.to_le_bytes()); // RVA-based
            data[d + 4..d + 8].copy_from_slice(&rva.to_le_bytes());
        }
        data.extend_from_slice(extra);

        let mut b = vec![0u8; raw];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3C..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
        b[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
        let coff = pe_off + 4;
        b[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        b[coff + 16..coff + 18].copy_from_slice(&(opt_size as u16).to_le_bytes());
        let opt = coff + 20;
        let magic: u16 = if pe32plus { 0x20b } else { 0x10b };
        b[opt..opt + 2].copy_from_slice(&magic.to_le_bytes());
        let (count_off, dirs_off) = if pe32plus {
            (opt + 108, opt + 112)
        } else {
            (opt + 92, opt + 96)
        };
        b[count_off..count_off + 4].copy_from_slice(&16u32.to_le_bytes());
        if !imports.is_empty() {
            let d = dirs_off + 8;
            b[d..d + 4].copy_from_slice(&(va + imp_desc as u32).to_le_bytes());
        }
        if !delay.is_empty() {
            let d = dirs_off + 13 * 8;
            b[d..d + 4].copy_from_slice(&(va + delay_desc as u32).to_le_bytes());
        }
        let s = table;
        b[s..s + 5].copy_from_slice(b".data");
        b[s + 8..s + 12].copy_from_slice(&(data.len() as u32).to_le_bytes());
        b[s + 12..s + 16].copy_from_slice(&va.to_le_bytes());
        b[s + 16..s + 20].copy_from_slice(&(data.len() as u32).to_le_bytes());
        b[s + 20..s + 24].copy_from_slice(&(raw as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_and_delay_imports_are_read_from_a_pe32_plus_image() {
        let img = fixture::pe(
            0x8664,
            true,
            &["KERNEL32.dll", "d3d12.dll", "DXGI.dll"],
            &["vulkan-1.dll"],
            b"",
        );
        let info = parse(&img).unwrap();
        assert_eq!(info.machine, Some(Machine::X64));
        assert_eq!(info.imports, ["kernel32.dll", "d3d12.dll", "dxgi.dll"]);
        assert_eq!(info.delay_imports, ["vulkan-1.dll"]);
        assert!(info.links("D3D12.DLL") && info.links("vulkan-1.dll") && !info.links("d3d11.dll"));
    }

    #[test]
    fn a_32_bit_image_is_told_apart() {
        let img = fixture::pe(0x014c, false, &["d3d9.dll"], &[], b"");
        let info = parse(&img).unwrap();
        assert_eq!(info.machine, Some(Machine::X86));
        assert_eq!(info.imports, ["d3d9.dll"]);
    }

    #[test]
    fn garbage_and_truncation_are_errors_not_panics() {
        assert_eq!(parse(b""), Err(PeError::NotPe));
        assert_eq!(parse(b"#!/bin/sh\n"), Err(PeError::NotPe));
        let img = fixture::pe(0x8664, true, &["d3d11.dll"], &[], b"");
        // Every prefix of a valid image either parses or fails cleanly.
        for cut in [0x40, 0x84, 0x90, 0x100, 0x200, 0x3ff, 0x410] {
            let _ = parse(&img[..cut.min(img.len())]);
        }
        let mut bad_magic = img.clone();
        bad_magic[0x80 + 24] = 0x33;
        assert_eq!(parse(&bad_magic), Err(PeError::UnknownFormat));
    }

    #[test]
    fn a_name_pointing_outside_the_file_is_skipped() {
        let mut img = fixture::pe(0x8664, true, &["d3d11.dll", "dxgi.dll"], &[], b"");
        // Point the first descriptor's name far past the end.
        let first = 0x400 + 12;
        img[first..first + 4].copy_from_slice(&0x00F0_0000u32.to_le_bytes());
        assert_eq!(parse(&img).unwrap().imports, ["dxgi.dll"]);
    }

    /// A `VS_VERSIONINFO` header followed by `VS_FIXEDFILEINFO` for `version`.
    fn version_block(version: [u16; 4]) -> Vec<u8> {
        let mut b = vec![0u8; 6]; // wLength, wValueLength, wType
        b.extend(
            "VS_VERSION_INFO\0"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        b.extend([0u8; 2]); // padding to a DWORD boundary
        b.extend(0xFEEF_04BDu32.to_le_bytes());
        b.extend(0x0001_0000u32.to_le_bytes());
        b.extend(((u32::from(version[0]) << 16) | u32::from(version[1])).to_le_bytes());
        b.extend(((u32::from(version[2]) << 16) | u32::from(version[3])).to_le_bytes());
        b
    }

    #[test]
    fn the_file_version_comes_from_the_version_resource() {
        let img = fixture::pe(
            0x8664,
            true,
            &["kernel32.dll"],
            &[],
            &version_block([310, 2, 1, 0]),
        );
        assert_eq!(file_version(&img).as_deref(), Some("310.2.1.0"));
        assert_eq!(file_version(b"no version here"), None);
    }

    #[test]
    fn a_stray_signature_before_the_real_block_is_ignored() {
        // The signature bytes on their own, aligned, with junk around them —
        // as found in NVIDIA's DLSS runtime — must not be read as a version.
        let mut extra = vec![0u8; 8];
        extra.extend(0xFEEF_04BDu32.to_le_bytes());
        extra.extend([0xAA; 16]);
        extra.extend(version_block([3, 7, 10, 2]));
        let img = fixture::pe(0x8664, true, &["kernel32.dll"], &[], &extra);
        assert_eq!(file_version(&img).as_deref(), Some("3.7.10.2"));
    }

    #[test]
    fn the_version_is_read_from_the_resource_directory_of_a_file() {
        let mut img = fixture::pe(
            0x8664,
            true,
            &["kernel32.dll"],
            &[],
            &version_block([1, 2, 3, 4]),
        );
        // Point data directory 2 (resources) at the whole section.
        let dirs = 0x80 + 24 + 112;
        img[dirs + 16..dirs + 20].copy_from_slice(&0x1000u32.to_le_bytes());
        let len = u32::try_from(img.len() - 0x400).unwrap();
        img[dirs + 20..dirs + 24].copy_from_slice(&len.to_le_bytes());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.dll");
        std::fs::write(&path, &img).unwrap();
        assert_eq!(read_file_version(&path).as_deref(), Some("1.2.3.4"));
        // Without a resource directory there is nothing to read.
        let plain = dir.path().join("plain.dll");
        std::fs::write(
            &plain,
            fixture::pe(0x8664, true, &["kernel32.dll"], &[], b""),
        )
        .unwrap();
        assert_eq!(read_file_version(&plain), None);
    }

    #[test]
    fn markers_are_found_in_ascii_and_in_utf16_version_resources() {
        let wide: Vec<u8> = "ReShade"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let img = fixture::pe(0x8664, true, &["kernel32.dll"], &[], &wide);
        assert!(contains_marker(&img, "ReShade"));
        assert!(!contains_marker(&img, "OptiScaler"));
        assert!(contains_marker(b"...OptiScaler v0.7...", "OptiScaler"));
    }
}
