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
    let Headers {
        machine,
        sections,
        dirs_off,
        dirs_count,
    } = headers(b)?;

    let dir = |index: usize| -> Option<u32> {
        (index < dirs_count)
            .then(|| u32_at(b, dirs_off + index * 8).ok())
            .flatten()
            .filter(|&rva| rva != 0)
    };

    let mut imports = Vec::new();
    if let Some(start) = dir(1).and_then(|rva| rva_to_offset(&sections, rva)) {
        for i in 0..MAX_DESCRIPTORS {
            let d = start + i * 20;
            let Some(desc) = b.get(d..d + 20) else { break };
            if desc.iter().all(|&x| x == 0) {
                break;
            }
            let name_rva = u32_at(b, d + 12)?;
            if let Some(name) = rva_to_offset(&sections, name_rva).and_then(|o| c_string(b, o)) {
                imports.push(name);
            }
        }
    }

    let mut delay_imports = Vec::new();
    if let Some(start) = dir(13).and_then(|rva| rva_to_offset(&sections, rva)) {
        for i in 0..MAX_DESCRIPTORS {
            let d = start + i * 32;
            let (Ok(attributes), Ok(name_rva)) = (u32_at(b, d), u32_at(b, d + 4)) else {
                break;
            };
            if name_rva == 0 {
                break;
            }
            // Attribute bit 0 clear is the pre-VC7 layout, where fields are
            // virtual addresses rather than RVAs; too old to matter for games
            // that have a modern upscaler, and not worth guessing an image base.
            if attributes & 1 == 0 {
                continue;
            }
            if let Some(name) = rva_to_offset(&sections, name_rva).and_then(|o| c_string(b, o)) {
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

/// Parse a PE file on disk, reading no more than `limit` bytes of it.
///
/// Import tables of game executables sit well within their first few
/// megabytes; a limit keeps a 200 MB executable from being read whole.
///
/// # Errors
/// Returns an error if the file cannot be read or is not a readable PE.
pub fn parse_file(path: &Path, limit: u64) -> anyhow::Result<PeInfo> {
    let bytes = read_prefix(path, limit)?;
    Ok(parse(&bytes)?)
}

/// The file version of a PE file on disk, read from its resource section only.
///
/// Version resources live in `.rsrc`, usually at the end of the file — past
/// the first 64 MB of Intel's 77 MB XeSS runtime, for one. The headers say
/// where the resource directory is, so only that span is read (capped at
/// 32 MB), not the whole file.
#[must_use]
pub fn read_file_version(path: &Path) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let head = read_prefix(path, 64 * 1024).ok()?;
    let h = headers(&head).ok()?;
    if h.dirs_count <= 2 {
        return None;
    }
    let rva = u32_at(&head, h.dirs_off + 16).ok()?;
    let size = u32_at(&head, h.dirs_off + 20).ok()?;
    let offset = rva_to_offset(&h.sections, rva)?;
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset as u64)).ok()?;
    let mut span = Vec::new();
    file.take(u64::from(size).min(32 << 20))
        .read_to_end(&mut span)
        .ok()?;
    file_version(&span)
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
