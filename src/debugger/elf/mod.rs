//! The functions and data an ELF64 little-endian file names, read from the
//! file, for the processor an [`ElfArch`] describes.
//!
//! An MFBASIC executable for Linux is a position-independent `ET_DYN` image at
//! link base 0 (`./mfb spec linker linux-aarch64`): its imports are the
//! undefined entries of the dynamic symbol table, each bound through a GOT
//! slot that a jump-slot or data-slot relocation (`R_AARCH64_JUMP_SLOT`,
//! `R_AARCH64_GLOB_DAT` on AArch64) names, and each called through a stub
//! (on AArch64 the 12 bytes `adrp x16` / `ldr x17, [x16]` / `br x17`) the
//! linker appends to the end of the code. A `--debug` build adds
//! a section table with a `.symtab` of every function and data object; a build
//! without it has no section table at all, so the dynamic table is reached
//! through `PT_DYNAMIC`, as the loader reaches it.
//!
//! A shared library is read the same way: its exports are the defined entries
//! of its dynamic symbol table (and of `.symtab` when it was not stripped), and
//! its `.plt` entries are named by the jump-slot relocations in order.
//!
//! The input is a file on disk and is untrusted: every offset is
//! bounds-checked, and a file whose headers do not fit is refused.

use super::arch::ElfArch;
use crate::os::inspect::{c_bytes, range, u32_at, u64_at};

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
/// `e_type`: an executable at a fixed address, and a position-independent
/// image (a PIE executable or a shared library).
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
/// `p_flags`: executable.
const PF_X: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_STRSZ: u64 = 10;
const DT_SONAME: u64 = 14;
const DT_JMPREL: u64 = 23;
const DT_GNU_HASH: u64 = 0x6fff_fef5;
const SYMBOL_SIZE: usize = 24;
const RELA_SIZE: usize = 24;
/// `st_info` types: an object, a function, and an indirect function.
const STT_OBJECT: u8 = 1;
const STT_FUNC: u8 = 2;
const STT_GNU_IFUNC: u8 = 10;
/// `st_info` bindings that export a symbol.
const STB_GLOBAL: u8 = 1;
const STB_WEAK: u8 = 2;
/// A library's `.plt`: the header, then one entry per jump-slot relocation.
const PLT_HEADER: u64 = 32;
const PLT_ENTRY: u64 = 16;
/// The most dynamic symbols a hash table is believed to count.
const MAX_DYNAMIC_SYMBOLS: usize = 1 << 20;

/// One loadable segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) vaddr: u64,
    pub(crate) memsz: u64,
    pub(crate) offset: u64,
    pub(crate) filesz: u64,
    pub(crate) executable: bool,
}

/// One symbol-table entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Symbol {
    pub(crate) name: String,
    /// The link-time address.
    pub(crate) value: u64,
    pub(crate) size: u64,
    /// `STT_*`.
    pub(crate) kind: u8,
    /// `STB_GLOBAL` or `STB_WEAK`.
    pub(crate) exported: bool,
    /// Not `SHN_UNDEF`.
    pub(crate) defined: bool,
}

impl Symbol {
    fn is_function(&self) -> bool {
        matches!(self.kind, STT_FUNC | STT_GNU_IFUNC)
    }
}

/// What [`read`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Elf {
    /// `ET_DYN` (relocatable at a slide) or `ET_EXEC`.
    pub(crate) position_independent: bool,
    /// The link-time entry point.
    pub(crate) entry: u64,
    /// The link-time address of the program headers, which the loader
    /// reports (`AT_PHDR`) at its live address.
    pub(crate) program_headers: Option<u64>,
    pub(crate) segments: Vec<Segment>,
    pub(crate) interpreter: Option<String>,
    /// `DT_NEEDED`, in order.
    pub(crate) needed: Vec<String>,
    pub(crate) soname: Option<String>,
    /// `.symtab`, in table order (empty when the file has none).
    pub(crate) symbols: Vec<Symbol>,
    /// The dynamic symbol table, index 0 (the null symbol) included.
    pub(crate) dynamic: Vec<Symbol>,
    /// Each GOT slot a relocation binds: the dynamic symbol's index and the
    /// slot's link-time address.
    pub(crate) slots: Vec<(usize, u64)>,
    /// The jump-slot relocations' symbols, in table order.
    pub(crate) plt_order: Vec<usize>,
    /// `.plt`'s link-time start, when a section table names it.
    pub(crate) plt: Option<(u64, u64)>,
}

/// The functions an MFBASIC executable defines and imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Executable {
    pub(crate) elf: Elf,
    pub(crate) functions: Vec<FileFunction>,
    /// Every data object the file defines, as `(name, link-time address)`.
    pub(crate) data: Vec<(String, u64)>,
    /// Each import stub: the import's symbol, the stub's link-time address,
    /// and its size.
    pub(crate) stubs: Vec<(String, u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileFunction {
    pub(crate) name: String,
    pub(crate) location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    /// A function this file defines, at its link-time address.
    Defined { address: u64, size: u64 },
    /// An import, with the link-time address of the GOT slot the loader binds
    /// it through, when a relocation names one.
    Import(Option<u64>),
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

/// The file bytes a link-time address range is loaded from.
fn at_address<'a>(
    bytes: &'a [u8],
    segments: &[Segment],
    address: u64,
    length: u64,
) -> Option<&'a [u8]> {
    let segment = segments.iter().find(|segment| {
        segment.vaddr <= address
            && address
                .checked_add(length)
                .is_some_and(|end| end <= segment.vaddr.saturating_add(segment.filesz))
    })?;
    range(bytes, segment.offset + (address - segment.vaddr), length)
}

/// The NUL-terminated string at `offset` in `table`.
fn string(table: &[u8], offset: u64) -> Option<String> {
    let start = usize::try_from(offset).ok()?;
    Some(String::from_utf8_lossy(c_bytes(table.get(start..)?)).into_owned())
}

fn symbol(entry: &[u8], strings: &[u8]) -> Option<Symbol> {
    let info = *entry.get(4)?;
    Some(Symbol {
        name: string(strings, u64::from(u32_at(entry, 0)?)).unwrap_or_default(),
        value: u64_at(entry, 8)?,
        size: u64_at(entry, 16)?,
        kind: info & 0xf,
        exported: matches!(info >> 4, STB_GLOBAL | STB_WEAK),
        defined: u16_at(entry, 6)? != 0,
    })
}

/// How many entries a `DT_GNU_HASH` table's symbol table has: one past the
/// last index any chain reaches.
fn gnu_hash_count(bytes: &[u8], segments: &[Segment], address: u64) -> Option<usize> {
    let header = at_address(bytes, segments, address, 16)?;
    let buckets = u64::from(u32_at(header, 0)?);
    let offset = u64::from(u32_at(header, 4)?);
    let bloom = u64::from(u32_at(header, 8)?);
    let table = address
        .checked_add(16)?
        .checked_add(bloom.checked_mul(8)?)?;
    let bucket_bytes = at_address(bytes, segments, table, buckets.checked_mul(4)?)?;
    let last = bucket_bytes
        .chunks_exact(4)
        .filter_map(|word| u32_at(word, 0))
        .max()
        .unwrap_or(0);
    if last < offset as u32 {
        return Some(offset as usize);
    }
    let chains = table.checked_add(buckets * 4)?;
    let mut index = u64::from(last);
    loop {
        let word = at_address(bytes, segments, chains + (index - offset) * 4, 4)?;
        if u32_at(word, 0)? & 1 == 1 {
            return usize::try_from(index + 1).ok();
        }
        index += 1;
        if index as usize > MAX_DYNAMIC_SYMBOLS {
            return None;
        }
    }
}

/// Read the ELF64 little-endian `bytes` for the processor `arch` describes,
/// or `None` when it is not one or its headers and tables do not fit the file.
pub(crate) fn read(bytes: &[u8], arch: &ElfArch) -> Option<Elf> {
    if bytes.get(0..4)? != ELF_MAGIC
        || bytes.get(4)? != &ELFCLASS64
        || bytes.get(5)? != &ELFDATA2LSB
    {
        return None;
    }
    let kind = u16_at(bytes, 16)?;
    if !matches!(kind, ET_EXEC | ET_DYN) || u16_at(bytes, 18)? != arch.machine {
        return None;
    }
    let entry = u64_at(bytes, 24)?;
    let program_offset = u64_at(bytes, 32)?;
    let section_offset = u64_at(bytes, 40)?;
    let program_size = u64::from(u16_at(bytes, 54)?);
    let program_count = u64::from(u16_at(bytes, 56)?);
    let section_size = u64::from(u16_at(bytes, 58)?);
    let section_count = u64::from(u16_at(bytes, 60)?);
    let section_names = usize::from(u16_at(bytes, 62)?);
    let headers = range(
        bytes,
        program_offset,
        program_size.checked_mul(program_count)?,
    )?;
    let mut segments = Vec::new();
    let (mut dynamic_range, mut interpreter, mut program_headers) = (None, None, None);
    for header in headers.chunks_exact(usize::try_from(program_size).ok()?.max(56)) {
        let kind = u32_at(header, 0)?;
        let flags = u32_at(header, 4)?;
        let offset = u64_at(header, 8)?;
        let vaddr = u64_at(header, 16)?;
        let filesz = u64_at(header, 32)?;
        let memsz = u64_at(header, 40)?;
        match kind {
            PT_LOAD => segments.push(Segment {
                vaddr,
                memsz,
                offset,
                filesz,
                executable: flags & PF_X != 0,
            }),
            PT_DYNAMIC => dynamic_range = Some((offset, filesz)),
            PT_INTERP => {
                interpreter = range(bytes, offset, filesz)
                    .map(|text| String::from_utf8_lossy(c_bytes(text)).into_owned())
            }
            PT_PHDR => program_headers = Some(vaddr),
            _ => {}
        }
    }
    // Without PT_PHDR, the headers are where the load that holds their file
    // bytes maps them.
    let program_headers = program_headers.or_else(|| {
        segments
            .iter()
            .find(|segment| {
                segment.offset <= program_offset
                    && program_offset < segment.offset.saturating_add(segment.filesz)
            })
            .map(|segment| segment.vaddr + (program_offset - segment.offset))
    });

    let mut elf = Elf {
        position_independent: kind == ET_DYN,
        entry,
        program_headers,
        segments,
        interpreter,
        needed: Vec::new(),
        soname: None,
        symbols: Vec::new(),
        dynamic: Vec::new(),
        slots: Vec::new(),
        plt_order: Vec::new(),
        plt: None,
    };
    if let Some((offset, size)) = dynamic_range {
        read_dynamic(bytes, &mut elf, range(bytes, offset, size)?, arch)?;
    }

    // The section table, when there is one: `.symtab` and `.plt`.
    if section_offset != 0 && section_count != 0 {
        let table = range(
            bytes,
            section_offset,
            section_size.checked_mul(section_count)?,
        )?;
        let entry_size = usize::try_from(section_size).ok()?.max(64);
        let sections = table.chunks_exact(entry_size).collect::<Vec<_>>();
        let names = sections
            .get(section_names)
            .and_then(|header| range(bytes, u64_at(header, 24)?, u64_at(header, 32)?));
        for header in &sections {
            let kind = u32_at(header, 4)?;
            if kind == SHT_SYMTAB {
                let offset = u64_at(header, 24)?;
                let size = u64_at(header, 32)?;
                let link = usize::try_from(u32_at(header, 40)?).ok()?;
                let strings = sections
                    .get(link)
                    .and_then(|header| range(bytes, u64_at(header, 24)?, u64_at(header, 32)?))?;
                let entries = range(bytes, offset, size)?;
                elf.symbols = entries
                    .chunks_exact(SYMBOL_SIZE)
                    .skip(1)
                    .filter_map(|entry| symbol(entry, strings))
                    .collect();
            }
            let name = names
                .zip(u32_at(header, 0))
                .and_then(|(names, at)| string(names, u64::from(at)));
            if name.as_deref() == Some(".plt") {
                elf.plt = Some((u64_at(header, 16)?, u64_at(header, 32)?));
            }
        }
    }
    Some(elf)
}

/// `DT_*` from the `.dynamic` bytes: the libraries needed, the soname, the
/// dynamic symbols, and the GOT slots relocations bind.
fn read_dynamic(bytes: &[u8], elf: &mut Elf, dynamic: &[u8], arch: &ElfArch) -> Option<()> {
    let mut tags = Vec::new();
    for entry in dynamic.chunks_exact(16) {
        let tag = u64_at(entry, 0)?;
        if tag == DT_NULL {
            break;
        }
        tags.push((tag, u64_at(entry, 8)?));
    }
    let value = |wanted: u64| tags.iter().find(|(tag, _)| *tag == wanted).map(|(_, v)| *v);
    let (Some(strtab), Some(strsz)) = (value(DT_STRTAB), value(DT_STRSZ)) else {
        return Some(());
    };
    let strings = at_address(bytes, &elf.segments, strtab, strsz)?;
    elf.needed = tags
        .iter()
        .filter(|(tag, _)| *tag == DT_NEEDED)
        .filter_map(|(_, offset)| string(strings, *offset))
        .collect();
    elf.soname = value(DT_SONAME).and_then(|offset| string(strings, offset));
    let count = match (value(DT_HASH), value(DT_GNU_HASH)) {
        (Some(hash), _) => {
            let header = at_address(bytes, &elf.segments, hash, 8)?;
            usize::try_from(u32_at(header, 4)?).ok()?
        }
        (None, Some(gnu)) => gnu_hash_count(bytes, &elf.segments, gnu)?,
        (None, None) => 0,
    };
    if count > MAX_DYNAMIC_SYMBOLS {
        return None;
    }
    if let (Some(symtab), true) = (value(DT_SYMTAB), count > 0) {
        let entries = at_address(bytes, &elf.segments, symtab, (count * SYMBOL_SIZE) as u64)?;
        elf.dynamic = entries
            .chunks_exact(SYMBOL_SIZE)
            .map(|entry| symbol(entry, strings))
            .collect::<Option<_>>()?;
    }
    let relocations = |address: Option<u64>, size: Option<u64>| -> Option<Vec<(u64, u32, usize)>> {
        let (Some(address), Some(size)) = (address, size) else {
            return Some(Vec::new());
        };
        let table = at_address(bytes, &elf.segments, address, size)?;
        table
            .chunks_exact(RELA_SIZE)
            .map(|entry| {
                let info = u64_at(entry, 8)?;
                Some((u64_at(entry, 0)?, info as u32, (info >> 32) as usize))
            })
            .collect()
    };
    let plt = relocations(value(DT_JMPREL), value(DT_PLTRELSZ))?;
    let other = relocations(value(DT_RELA), value(DT_RELASZ))?;
    elf.plt_order = plt
        .iter()
        .filter(|(_, kind, _)| *kind == arch.jump_slot)
        .map(|(_, _, index)| *index)
        .collect();
    for (slot, kind, index) in plt.into_iter().chain(other) {
        if (kind == arch.jump_slot || kind == arch.data_slot)
            && index != 0
            && index < elf.dynamic.len()
            && !elf.slots.contains(&(index, slot))
        {
            elf.slots.push((index, slot));
        }
    }
    Some(())
}

impl Elf {
    /// The executable segments' link-time ranges, `[start, end)`.
    pub(crate) fn code(&self) -> Vec<(u64, u64)> {
        self.segments
            .iter()
            .filter(|segment| segment.executable)
            .map(|segment| (segment.vaddr, segment.vaddr.saturating_add(segment.memsz)))
            .collect()
    }

    /// Every function the file defines, from `.symtab` and the dynamic symbol
    /// table, once per (name, address): `(name, link-time address, size,
    /// exported)`, by address.
    pub(crate) fn defined_functions(&self) -> Vec<(String, u64, u64, bool)> {
        let mut functions = self
            .dynamic
            .iter()
            .chain(&self.symbols)
            .filter(|symbol| symbol.defined && symbol.is_function() && !symbol.name.is_empty())
            .map(|symbol| {
                (
                    symbol.name.clone(),
                    symbol.value,
                    symbol.size,
                    symbol.exported,
                )
            })
            .collect::<Vec<_>>();
        functions.sort_by(|a, b| (a.1, &a.0, !a.3).cmp(&(b.1, &b.0, !b.3)));
        functions.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        functions
    }

    /// The `.plt` entry holding the link-time `address`: its start and the
    /// symbol its relocation names.
    pub(crate) fn plt_entry(&self, address: u64) -> Option<(u64, String)> {
        let (start, size) = self.plt?;
        if address < start + PLT_HEADER || address >= start + size {
            return None;
        }
        let index = (address - start - PLT_HEADER) / PLT_ENTRY;
        let symbol = self.plt_order.get(usize::try_from(index).ok()?)?;
        let name = self.dynamic.get(*symbol)?.name.clone();
        Some((start + PLT_HEADER + index * PLT_ENTRY, name))
    }
}

/// Read an MFBASIC executable's functions, data and import stubs: `.symtab`'s
/// functions in table order, then each undefined dynamic symbol (an import)
/// with the GOT slot a relocation binds it through.
pub(crate) fn executable(bytes: &[u8], arch: &ElfArch) -> Option<Executable> {
    let elf = read(bytes, arch)?;
    let mut functions = Vec::new();
    let mut data = Vec::new();
    for symbol in &elf.symbols {
        if !symbol.defined || symbol.name.is_empty() {
            continue;
        }
        match symbol.kind {
            STT_FUNC | STT_GNU_IFUNC => functions.push(FileFunction {
                name: symbol.name.clone(),
                location: Location::Defined {
                    address: symbol.value,
                    size: symbol.size,
                },
            }),
            STT_OBJECT => data.push((symbol.name.clone(), symbol.value)),
            _ => {}
        }
    }
    for (index, symbol) in elf.dynamic.iter().enumerate().skip(1) {
        if symbol.defined || symbol.name.is_empty() {
            continue;
        }
        let slot = elf
            .slots
            .iter()
            .find(|(symbol, _)| *symbol == index)
            .map(|(_, slot)| *slot);
        functions.push(FileFunction {
            name: symbol.name.clone(),
            location: Location::Import(slot),
        });
    }
    let stubs = stubs(bytes, &elf, arch);
    Some(Executable {
        elf,
        functions,
        data,
        stubs,
    })
}

/// The import stubs at the end of the code: walking back from the end of the
/// executable segment, every [`ElfArch::stub_size`] bytes that are a stub
/// ([`ElfArch::stub_slot`]) are one, named by the import whose GOT slot they
/// load (`crate::os::linux::link::emit_import_stub`).
fn stubs(bytes: &[u8], elf: &Elf, arch: &ElfArch) -> Vec<(String, u64, u64)> {
    let mut found = Vec::new();
    let Some(text) = elf.segments.iter().find(|segment| segment.executable) else {
        return found;
    };
    let end = text.vaddr + text.filesz;
    let size = arch.stub_size;
    let mut at = end;
    while at >= text.vaddr + size {
        let start = at - size;
        let Some(code) = at_address(bytes, &elf.segments, start, size) else {
            break;
        };
        let Some(slot) = (arch.stub_slot)(code, start) else {
            break;
        };
        let name = elf
            .slots
            .iter()
            .find(|(_, at)| *at == slot)
            .and_then(|(index, _)| elf.dynamic.get(*index))
            .map(|symbol| symbol.name.clone());
        let Some(name) = name else {
            break;
        };
        found.push((name, start, size));
        at = start;
    }
    found.reverse();
    found
}

#[cfg(test)]
mod tests;
