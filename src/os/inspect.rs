//! A bounds-checked reader for the executables the in-tree linkers write.
//!
//! It recognizes an ELF, Mach-O or PE image by magic and finds, through that
//! format's own tables, the `MFBasic\0` provenance note (`./mfb spec linker
//! provenance-marker`) and the optional `.mfbsign` signing section. Two callers
//! depend on it agreeing with itself: `mfb info`, which reports and verifies what
//! it finds, and the linkers, which seal the `contentSignature` over the very
//! range this reader will later locate (`crate::os::content_signature`).
//!
//! The input is untrusted. Every offset, length and count the file names is
//! bounds-checked before use; a file that does not fit its own headers is simply
//! not an MFBasic binary.

use std::ops::Range;

use crate::os::note::{MFB_NOTE_DESCRIPTOR_SIZE, MFB_NOTE_OWNER, MFB_NOTE_TYPE};

/// What [`inspect`] found in an image that carries the provenance note.
pub(crate) struct Binary {
    pub(crate) format: &'static str,
    pub(crate) arch: String,
    /// ELF only: whether the image names a program interpreter.
    pub(crate) linking: Option<Linking>,
    /// The shared libraries the image loads: ELF `DT_NEEDED`, Mach-O
    /// `LC_LOAD_DYLIB` (and its weak/re-export/upward forms), PE import DLLs.
    pub(crate) libraries: Vec<String>,
    /// Loader search paths for `dlopen`ed libraries: ELF `DT_RUNPATH`/`DT_RPATH`,
    /// Mach-O `LC_RPATH`. PE has none.
    pub(crate) search_paths: Vec<String>,
    pub(crate) compiler: String,
    /// The byte range of the `.mfbsign` section body, when the build was signed.
    pub(crate) signing: Option<Range<usize>>,
    /// Where the bytes a content signature covers end: the whole file, except on
    /// Mach-O, where the ad-hoc code signature (`LC_CODE_SIGNATURE`) that is
    /// computed after the seal starts.
    pub(crate) covered_end: usize,
}

pub(crate) enum Linking {
    Static,
    Dynamic(String),
}

/// Classify `bytes` by magic and read the provenance note out of whichever format
/// it is. `None` means "not an MFBasic binary": an unknown format, a header that
/// does not fit the file, or no `MFBasic\0` note carrying an `MFB1` descriptor.
pub(crate) fn inspect(bytes: &[u8]) -> Option<Binary> {
    let raw = if bytes.starts_with(b"\x7fELF") {
        inspect_elf(bytes)
    } else if u32_at(bytes, 0) == Some(MH_MAGIC_64) {
        inspect_mach_o(bytes)
    } else if bytes.starts_with(b"MZ") {
        inspect_pe(bytes)
    } else {
        None
    }?;
    Some(Binary {
        format: raw.format,
        arch: raw.arch,
        linking: raw.linking,
        libraries: raw.libraries,
        search_paths: raw.search_paths,
        compiler: compiler(raw.descriptor)?,
        signing: raw.signing.map(|blob| offset_of(bytes, blob)),
        covered_end: raw.covered_end,
    })
}

/// A format reader's findings, before the descriptor is decoded.
struct RawBinary<'a> {
    format: &'static str,
    arch: String,
    linking: Option<Linking>,
    libraries: Vec<String>,
    search_paths: Vec<String>,
    descriptor: &'a [u8],
    signing: Option<&'a [u8]>,
    covered_end: usize,
}

/// The compiler named by a provenance descriptor (`src/os/note.rs`), or `None`
/// when the bytes are not an `MFB1` descriptor at all.
pub(crate) fn compiler(descriptor: &[u8]) -> Option<String> {
    if descriptor.len() < MFB_NOTE_DESCRIPTOR_SIZE || !descriptor.starts_with(b"MFB1") {
        return None;
    }
    let version = u16_at(descriptor, 4)?;
    if version != 1 {
        return Some(format!("unknown (marker descriptor version {version})"));
    }
    Some(format!(
        "mfb {}.{}.{}",
        u16_at(descriptor, 8)?,
        u16_at(descriptor, 10)?,
        u16_at(descriptor, 12)?
    ))
}

/// The range `part` occupies inside `bytes`; `part` is always a sub-slice of it.
fn offset_of(bytes: &[u8], part: &[u8]) -> Range<usize> {
    let start = part.as_ptr() as usize - bytes.as_ptr() as usize;
    start..start + part.len()
}

// ---------------------------------------------------------------------------
// ELF
// ---------------------------------------------------------------------------

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_NOTE: u32 = 4;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;
const DT_RPATH: u64 = 15;
const DT_RUNPATH: u64 = 29;
const ELF_PROGRAM_HEADER_SIZE: usize = 56;
const ELF_SECTION_HEADER_SIZE: usize = 64;

/// A 64-bit little-endian ELF: every in-tree Linux target.
fn inspect_elf(bytes: &[u8]) -> Option<RawBinary<'_>> {
    if bytes.get(4) != Some(&2) || bytes.get(5) != Some(&1) {
        return None;
    }
    let arch = match u16_at(bytes, 18)? {
        62 => "x86-64".to_string(),
        183 => "aarch64".to_string(),
        243 => "riscv64".to_string(),
        other => format!("unknown (e_machine {other})"),
    };
    let entries = table(
        bytes,
        u64_at(bytes, 32)?,
        u16_at(bytes, 54)?,
        u16_at(bytes, 56)?,
        ELF_PROGRAM_HEADER_SIZE,
    )?;
    let mut descriptor = None;
    let mut linking = Linking::Static;
    let mut loads = Vec::new();
    let mut dynamic = None;
    for entry in entries {
        let offset = u64_at(entry, 8)?;
        let file_size = u64_at(entry, 32)?;
        let contents = range(bytes, offset, file_size);
        match (u32_at(entry, 0)?, contents) {
            (PT_NOTE, Some(notes)) if descriptor.is_none() => {
                descriptor = elf_note_descriptor(notes);
            }
            (PT_INTERP, Some(path)) => linking = Linking::Dynamic(c_string(path)),
            (PT_LOAD, _) => loads.push((u64_at(entry, 16)?, offset, file_size)),
            (PT_DYNAMIC, Some(section)) => dynamic = Some(section),
            _ => {}
        }
    }
    let (libraries, search_paths) = dynamic
        .map(|section| elf_dynamic_names(bytes, section, &loads))
        .unwrap_or_default();
    Some(RawBinary {
        format: "ELF",
        arch,
        linking: Some(linking),
        libraries,
        search_paths,
        descriptor: descriptor?,
        signing: elf_section(bytes, b".mfbsign"),
        covered_end: bytes.len(),
    })
}

/// The descriptor of the `MFBasic\0` note inside one `PT_NOTE` segment's bytes.
fn elf_note_descriptor(notes: &[u8]) -> Option<&[u8]> {
    let mut at = 0usize;
    loop {
        let header = notes.get(at..at.checked_add(12)?)?;
        let name_size = u64::from(u32_at(header, 0)?);
        let desc_size = u64::from(u32_at(header, 4)?);
        let name_start = at + 12;
        let name = range(notes, name_start as u64, name_size)?;
        let desc_start = name_start.checked_add(align4(name_size)?)?;
        let desc = range(notes, desc_start as u64, desc_size)?;
        if name == MFB_NOTE_OWNER && u32_at(header, 8)? == MFB_NOTE_TYPE {
            return Some(desc);
        }
        at = desc_start.checked_add(align4(desc_size)?)?;
    }
}

/// The `DT_NEEDED` libraries and `DT_RUNPATH`/`DT_RPATH` search paths of a
/// `PT_DYNAMIC` segment, resolved through `DT_STRTAB` the way the loader does:
/// the table's virtual address maps to a file offset through the `PT_LOAD`
/// segments (`(vaddr, offset, file size)`).
fn elf_dynamic_names(
    bytes: &[u8],
    dynamic: &[u8],
    loads: &[(u64, u64, u64)],
) -> (Vec<String>, Vec<String>) {
    let mut strtab = None;
    let mut strsz = None;
    let mut needed = Vec::new();
    let mut paths = Vec::new();
    for entry in dynamic.chunks_exact(16) {
        let (Some(tag), Some(value)) = (u64_at(entry, 0), u64_at(entry, 8)) else {
            break;
        };
        match tag {
            DT_NULL => break,
            DT_STRTAB => strtab = Some(value),
            DT_STRSZ => strsz = Some(value),
            DT_NEEDED => needed.push(value),
            DT_RPATH | DT_RUNPATH => paths.push(value),
            _ => {}
        }
    }
    let table = strtab
        .and_then(|vaddr| {
            loads
                .iter()
                .find(|(base, _, size)| vaddr >= *base && vaddr - base < *size)
                .and_then(|(base, offset, _)| (vaddr - base).checked_add(*offset))
        })
        .and_then(|offset| match strsz {
            Some(size) => range(bytes, offset, size),
            None => bytes.get(usize::try_from(offset).ok()?..),
        });
    let Some(table) = table else {
        return (Vec::new(), Vec::new());
    };
    let name = |offset: u64| table.get(usize::try_from(offset).ok()?..).map(c_string);
    (
        needed.into_iter().filter_map(name).collect(),
        paths.into_iter().filter_map(name).collect(),
    )
}

/// The contents of the section named `wanted`, located through the section
/// header table and its `.shstrtab`.
fn elf_section<'a>(bytes: &'a [u8], wanted: &[u8]) -> Option<&'a [u8]> {
    let sections = table(
        bytes,
        u64_at(bytes, 40)?,
        u16_at(bytes, 58)?,
        u16_at(bytes, 60)?,
        ELF_SECTION_HEADER_SIZE,
    )?;
    let names = sections.get(usize::from(u16_at(bytes, 62)?))?;
    let names = range(bytes, u64_at(names, 24)?, u64_at(names, 32)?)?;
    sections.into_iter().find_map(|section| {
        let name = names.get(usize::try_from(u32_at(section, 0)?).ok()?..)?;
        if c_bytes(name) != wanted {
            return None;
        }
        range(bytes, u64_at(section, 24)?, u64_at(section, 32)?)
    })
}

/// An ELF header table: `count` records of `entry_size` bytes at `offset`. An
/// entry size below the fields this reader uses, or a table that does not fit
/// the file, answers `None`; an absent table (`offset` 0) answers empty.
fn table(
    bytes: &[u8],
    offset: u64,
    entry_size: u16,
    count: u16,
    minimum_size: usize,
) -> Option<Vec<&[u8]>> {
    if offset == 0 || count == 0 {
        return Some(Vec::new());
    }
    let entry_size = usize::from(entry_size);
    if entry_size < minimum_size {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    (0..usize::from(count))
        .map(|index| {
            let base = start.checked_add(index * entry_size)?;
            bytes.get(base..base.checked_add(minimum_size)?)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Mach-O
// ---------------------------------------------------------------------------

pub(crate) const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_NOTE: u32 = 0x31;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_RPATH: u32 = 0x8000_001c;
const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
const SEGMENT_COMMAND_SIZE: usize = 72;
const SECTION_SIZE: usize = 80;

/// A 64-bit little-endian Mach-O: the macOS target.
fn inspect_mach_o(bytes: &[u8]) -> Option<RawBinary<'_>> {
    let arch = match u32_at(bytes, 4)? {
        0x0100_000c => "aarch64".to_string(),
        0x0100_0007 => "x86-64".to_string(),
        other => format!("unknown (cputype {other:#x})"),
    };
    let mut descriptor = None;
    let mut signing = None;
    let mut covered_end = bytes.len();
    let mut libraries = Vec::new();
    let mut search_paths = Vec::new();
    let mut at = 32usize;
    for _ in 0..u32_at(bytes, 16)? {
        let size = usize::try_from(u32_at(bytes, at.checked_add(4)?)?).ok()?;
        if size < 8 {
            return None;
        }
        let command = bytes.get(at..at.checked_add(size)?)?;
        match u32_at(command, 0)? {
            LC_NOTE if descriptor.is_none() && command.len() >= 40 => {
                if c_bytes(&command[8..24]) == &MFB_NOTE_OWNER[..MFB_NOTE_OWNER.len() - 1] {
                    descriptor = range(bytes, u64_at(command, 24)?, u64_at(command, 32)?);
                }
            }
            LC_SEGMENT_64 if command.len() >= SEGMENT_COMMAND_SIZE => {
                let sections = command[SEGMENT_COMMAND_SIZE..].chunks_exact(SECTION_SIZE);
                for section in sections.take(usize::try_from(u32_at(command, 64)?).ok()?) {
                    if c_bytes(&section[0..16]) == b".mfbsign"
                        && c_bytes(&section[16..32]) == b"__MFB"
                    {
                        signing =
                            range(bytes, u64::from(u32_at(section, 48)?), u64_at(section, 40)?);
                    }
                }
            }
            LC_CODE_SIGNATURE if command.len() >= 16 => {
                covered_end = usize::try_from(u32_at(command, 8)?).ok()?.min(bytes.len());
            }
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LOAD_UPWARD_DYLIB => {
                libraries.extend(mach_o_command_string(command));
            }
            LC_RPATH => search_paths.extend(mach_o_command_string(command)),
            _ => {}
        }
        at += size;
    }
    Some(RawBinary {
        format: "Mach-O",
        arch,
        linking: None,
        libraries,
        search_paths,
        descriptor: descriptor?,
        signing,
        covered_end,
    })
}

/// The string a dylib or rpath load command carries: its `u32` offset (from the
/// command's start) at byte 8, NUL-terminated inside the command.
fn mach_o_command_string(command: &[u8]) -> Option<String> {
    let offset = usize::try_from(u32_at(command, 8)?).ok()?;
    command.get(offset..).map(c_string)
}

// ---------------------------------------------------------------------------
// PE
// ---------------------------------------------------------------------------

const PE_SECTION_HEADER_SIZE: usize = 40;
const PE32_PLUS_MAGIC: u16 = 0x20b;
const IMPORT_DESCRIPTOR_SIZE: usize = 20;

/// A PE32+ image: the Windows target.
fn inspect_pe(bytes: &[u8]) -> Option<RawBinary<'_>> {
    let pe = usize::try_from(u32_at(bytes, 0x3c)?).ok()?;
    let coff = bytes.get(pe..pe.checked_add(24)?)?;
    if !coff.starts_with(b"PE\0\0") {
        return None;
    }
    let arch = match u16_at(coff, 4)? {
        0x8664 => "x86-64".to_string(),
        0xaa64 => "aarch64".to_string(),
        other => format!("unknown (machine {other:#x})"),
    };
    let section_count = usize::from(u16_at(coff, 6)?);
    let optional_size = usize::from(u16_at(coff, 20)?);
    let table_start = pe.checked_add(24 + optional_size)?;
    let optional = bytes.get(pe + 24..table_start)?;
    let mut descriptor = None;
    let mut signing = None;
    // `(virtual address, virtual size, raw pointer, raw size)` per section, for
    // mapping the import directory's RVAs to file offsets.
    let mut sections = Vec::new();
    for index in 0..section_count {
        let base = table_start.checked_add(index * PE_SECTION_HEADER_SIZE)?;
        let header = bytes.get(base..base.checked_add(PE_SECTION_HEADER_SIZE)?)?;
        let virtual_size = u32_at(header, 8)?;
        let raw_size = u32_at(header, 16)?;
        sections.push((
            u32_at(header, 12)?,
            virtual_size,
            u32_at(header, 20)?,
            raw_size,
        ));
        // The raw body is FILE_ALIGNMENT-padded; the virtual size is the exact
        // payload length the linker wrote.
        let length = if virtual_size == 0 {
            raw_size
        } else {
            virtual_size.min(raw_size)
        };
        let body = range(bytes, u64::from(u32_at(header, 20)?), u64::from(length));
        match c_bytes(&header[0..8]) {
            b".mfbnote" => {
                descriptor = body
                    .and_then(|body| body.strip_prefix(MFB_NOTE_OWNER.as_slice()))
                    .or(descriptor);
            }
            b".mfbsign" => signing = body.or(signing),
            _ => {}
        }
    }
    Some(RawBinary {
        format: "PE",
        arch,
        linking: None,
        libraries: pe_import_dlls(bytes, optional, &sections),
        search_paths: Vec::new(),
        descriptor: descriptor?,
        signing,
        covered_end: bytes.len(),
    })
}

/// The DLL names in a PE32+ image's import directory (data directory `[1]`):
/// one 20-byte descriptor per DLL, its name RVA at byte 12, ended by an all-zero
/// descriptor.
fn pe_import_dlls(bytes: &[u8], optional: &[u8], sections: &[(u32, u32, u32, u32)]) -> Vec<String> {
    let rva_to_offset = |rva: u32| {
        sections
            .iter()
            .find(|(address, _, _, raw_size)| rva >= *address && rva - address < *raw_size)
            .map(|(address, _, raw_pointer, _)| u64::from(rva - address) + u64::from(*raw_pointer))
    };
    let directory = (|| {
        if u16_at(optional, 0)? != PE32_PLUS_MAGIC || u32_at(optional, 108)? < 2 {
            return None;
        }
        let rva = u32_at(optional, 120)?;
        if rva == 0 {
            return None;
        }
        usize::try_from(rva_to_offset(rva)?).ok()
    })();
    let Some(mut at) = directory else {
        return Vec::new();
    };
    let mut dlls = Vec::new();
    while let Some(entry) = at
        .checked_add(IMPORT_DESCRIPTOR_SIZE)
        .and_then(|end| bytes.get(at..end))
    {
        if entry.iter().all(|byte| *byte == 0) {
            break;
        }
        if let Some(name) = u32_at(entry, 12)
            .and_then(rva_to_offset)
            .and_then(|offset| bytes.get(usize::try_from(offset).ok()?..))
        {
            dlls.push(c_string(name));
        }
        at += IMPORT_DESCRIPTOR_SIZE;
    }
    dlls
}

// ---------------------------------------------------------------------------
// Bounds-checked byte access
// ---------------------------------------------------------------------------

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// `length` bytes at `offset`, or `None` when that range leaves the file.
fn range(bytes: &[u8], offset: u64, length: u64) -> Option<&[u8]> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(length).ok()?)?;
    bytes.get(start..end)
}

fn align4(value: u64) -> Option<usize> {
    usize::try_from(value.checked_add(3)? & !3).ok()
}

/// The bytes before the first NUL (all of them when there is none).
fn c_bytes(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    &bytes[..end]
}

fn c_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(c_bytes(bytes)).into_owned()
}
