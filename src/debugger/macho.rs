//! The functions a Mach-O executable names, read from its own symbol table.
//!
//! Two kinds of `nlist_64` entry are functions: a symbol defined in a section
//! that holds instructions, and an undefined external (an import the loader
//! binds). Each import record of an MFBASIC executable has its own undefined
//! entry and its own non-lazy pointer slot (`./mfb spec linker import-selection`),
//! found through the indirect symbol table; the slot is where the loader writes
//! the function's address. Entries are kept in symbol-table order, repeats
//! included, so the listing matches `nm`.
//!
//! The input is the file on disk and is untrusted: every offset is
//! bounds-checked, and a file whose tables do not fit is refused.

use crate::os::inspect::{
    c_bytes, range, u32_at, u64_at, LC_SEGMENT_64, MH_MAGIC_64, SECTION_SIZE, SEGMENT_COMMAND_SIZE,
};

const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
/// `LC_MAIN`: the entry point, as an offset into the file (and so into
/// `__TEXT`, which starts at file offset 0).
const LC_MAIN: u32 = 0x8000_0028;
const NLIST_SIZE: usize = 16;
/// `n_type` bits: a debugger stab, the type field, and "external".
const N_STAB: u8 = 0xe0;
const N_TYPE: u8 = 0x0e;
const N_EXT: u8 = 0x01;
const N_UNDF: u8 = 0x0;
const N_SECT: u8 = 0xe;
/// Section flags: the section type, and the two "contains instructions" bits.
const SECTION_TYPE: u32 = 0xff;
const S_NON_LAZY_SYMBOL_POINTERS: u32 = 0x6;
const S_LAZY_SYMBOL_POINTERS: u32 = 0x7;
const S_SYMBOL_STUBS: u32 = 0x8;
const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x400;
/// Indirect-symbol entries that name no symbol.
const INDIRECT_SYMBOL_LOCAL: u32 = 0x8000_0000;
const INDIRECT_SYMBOL_ABS: u32 = 0x4000_0000;

/// What [`read`] found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Image {
    /// The address `__TEXT` (and so the Mach-O header) asks to load at; the
    /// live address minus this is the load offset.
    pub(crate) text_address: u64,
    pub(crate) functions: Vec<FileFunction>,
    /// Every other symbol the file defines (data such as the runtime's
    /// globals), as `(name, unslid address)`.
    pub(crate) data: Vec<(String, u64)>,
    /// The unslid address `LC_MAIN` names as the entry point.
    pub(crate) entry: Option<u64>,
    /// Each import stub (`__stubs`): the import's symbol, the stub's unslid
    /// address, and its size.
    pub(crate) stubs: Vec<(String, u64, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FileFunction {
    pub(crate) name: String,
    pub(crate) location: Location,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Location {
    /// A function this file defines, at its unslid address, running up to the
    /// next function in its section or the section's end.
    Defined { address: u64, size: u64 },
    /// An import, with the pointer slot the loader binds it through when the
    /// file has one.
    Import(Option<Slot>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    /// The slot's unslid address.
    pub(crate) address: u64,
    /// The slot's bytes in the file: what it holds until the loader binds it.
    pub(crate) unbound: [u8; 8],
}

struct Section {
    address: u64,
    size: u64,
    offset: u32,
    flags: u32,
    reserved1: u32,
    reserved2: u32,
}

/// Read the function symbols of the 64-bit Mach-O `bytes`, or `None` when its
/// headers or tables do not fit the file. A file with no symbol table names no
/// functions.
pub(crate) fn read(bytes: &[u8]) -> Option<Image> {
    if u32_at(bytes, 0)? != MH_MAGIC_64 {
        return None;
    }
    let mut sections = Vec::new();
    let mut text_address = None;
    let mut symtab = None;
    let mut indirect = None;
    let mut entry_offset = None;
    let mut at = 32usize;
    for _ in 0..u32_at(bytes, 16)? {
        let size = usize::try_from(u32_at(bytes, at.checked_add(4)?)?).ok()?;
        if size < 8 {
            return None;
        }
        let command = bytes.get(at..at.checked_add(size)?)?;
        match u32_at(command, 0)? {
            LC_SEGMENT_64 if command.len() >= SEGMENT_COMMAND_SIZE => {
                if c_bytes(&command[8..24]) == b"__TEXT" {
                    text_address = Some(u64_at(command, 24)?);
                }
                let count = usize::try_from(u32_at(command, 64)?).ok()?;
                let records = command.get(SEGMENT_COMMAND_SIZE..)?;
                if records.len() / SECTION_SIZE < count {
                    return None;
                }
                for record in records.chunks_exact(SECTION_SIZE).take(count) {
                    sections.push(Section {
                        address: u64_at(record, 32)?,
                        size: u64_at(record, 40)?,
                        offset: u32_at(record, 48)?,
                        flags: u32_at(record, 64)?,
                        reserved1: u32_at(record, 68)?,
                        reserved2: u32_at(record, 72)?,
                    });
                }
            }
            LC_SYMTAB if command.len() >= 24 => {
                symtab = Some((
                    u32_at(command, 8)?,
                    u32_at(command, 12)?,
                    u32_at(command, 16)?,
                    u32_at(command, 20)?,
                ));
            }
            LC_DYSYMTAB if command.len() >= 64 => {
                indirect = Some((u32_at(command, 56)?, u32_at(command, 60)?));
            }
            LC_MAIN if command.len() >= 16 => entry_offset = Some(u64_at(command, 8)?),
            _ => {}
        }
        at += size;
    }
    let text_address = text_address?;
    let entry = match entry_offset {
        Some(offset) => Some(text_address.checked_add(offset)?),
        None => None,
    };
    let Some((symoff, nsyms, stroff, strsize)) = symtab else {
        return Some(Image {
            text_address,
            functions: Vec::new(),
            data: Vec::new(),
            entry,
            stubs: Vec::new(),
        });
    };
    let symbols = range(
        bytes,
        u64::from(symoff),
        u64::from(nsyms).checked_mul(NLIST_SIZE as u64)?,
    )?;
    let strings = range(bytes, u64::from(stroff), u64::from(strsize))?;
    let slots = import_slots(bytes, &sections, indirect)?;

    // Code symbols as (entry in `functions`, section index, address), for sizing.
    let mut code = Vec::new();
    let mut functions = Vec::new();
    let mut data = Vec::new();
    for (index, entry) in symbols.chunks_exact(NLIST_SIZE).enumerate() {
        let kind = entry[4];
        if kind & N_STAB != 0 {
            continue;
        }
        let name = strings.get(usize::try_from(u32_at(entry, 0)?).ok()?..)?;
        let name = String::from_utf8_lossy(c_bytes(name)).into_owned();
        let location = match kind & N_TYPE {
            N_SECT => {
                let section_index = usize::from(entry[5]).checked_sub(1)?;
                let section = sections.get(section_index)?;
                let address = u64_at(entry, 8)?;
                if section.flags & (S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS) == 0 {
                    data.push((name, address));
                    continue;
                }
                code.push((functions.len(), section_index, address));
                Location::Defined { address, size: 0 }
            }
            N_UNDF if kind & N_EXT != 0 => Location::Import(
                slots
                    .iter()
                    .find(|(symbol, _)| *symbol == index)
                    .map(|(_, slot)| slot.clone()),
            ),
            _ => continue,
        };
        functions.push(FileFunction { name, location });
    }
    for &(at, section_index, address) in &code {
        let section = &sections[section_index];
        let end = code
            .iter()
            .filter(|(_, other, start)| *other == section_index && *start > address)
            .map(|(_, _, start)| *start)
            .min()
            .unwrap_or_else(|| section.address.saturating_add(section.size));
        functions[at].location = Location::Defined {
            address,
            size: end.saturating_sub(address),
        };
    }
    let stubs = import_stubs(bytes, &sections, indirect, symbols, strings)?;
    Some(Image {
        text_address,
        functions,
        data,
        entry,
        stubs,
    })
}

/// Each stub in a `S_SYMBOL_STUBS` section (`reserved2` bytes long), named by
/// the symbol its indirect-symbol entry selects.
fn import_stubs(
    bytes: &[u8],
    sections: &[Section],
    indirect: Option<(u32, u32)>,
    symbols: &[u8],
    strings: &[u8],
) -> Option<Vec<(String, u64, u64)>> {
    let Some((indirectoff, nindirect)) = indirect else {
        return Some(Vec::new());
    };
    let table = range(
        bytes,
        u64::from(indirectoff),
        u64::from(nindirect).checked_mul(4)?,
    )?;
    let mut stubs = Vec::new();
    for section in sections {
        if section.flags & SECTION_TYPE != S_SYMBOL_STUBS || section.reserved2 == 0 {
            continue;
        }
        let size = u64::from(section.reserved2);
        for stub in 0..section.size / size {
            let entry_index = u64::from(section.reserved1).checked_add(stub)?;
            let entry = u32_at(table, usize::try_from(entry_index.checked_mul(4)?).ok()?)?;
            if entry & (INDIRECT_SYMBOL_LOCAL | INDIRECT_SYMBOL_ABS) != 0 {
                continue;
            }
            let nlist = symbols.get(usize::try_from(entry).ok()?.checked_mul(NLIST_SIZE)?..)?;
            let name = strings.get(usize::try_from(u32_at(nlist, 0)?).ok()?..)?;
            stubs.push((
                String::from_utf8_lossy(c_bytes(name)).into_owned(),
                section.address.checked_add(stub.checked_mul(size)?)?,
                size,
            ));
        }
    }
    Some(stubs)
}

/// Each symbol-pointer slot, as `(symbol index, slot)`, from the pointer
/// sections and the indirect symbol table they index into.
fn import_slots(
    bytes: &[u8],
    sections: &[Section],
    indirect: Option<(u32, u32)>,
) -> Option<Vec<(usize, Slot)>> {
    let Some((indirectoff, nindirect)) = indirect else {
        return Some(Vec::new());
    };
    let table = range(
        bytes,
        u64::from(indirectoff),
        u64::from(nindirect).checked_mul(4)?,
    )?;
    let mut slots = Vec::new();
    for section in sections {
        let kind = section.flags & SECTION_TYPE;
        if kind != S_NON_LAZY_SYMBOL_POINTERS && kind != S_LAZY_SYMBOL_POINTERS {
            continue;
        }
        let contents = range(bytes, u64::from(section.offset), section.size)?;
        for (slot, unbound) in contents.chunks_exact(8).enumerate() {
            let entry_index = usize::try_from(section.reserved1).ok()?.checked_add(slot)?;
            let entry = u32_at(table, entry_index.checked_mul(4)?)?;
            if entry & (INDIRECT_SYMBOL_LOCAL | INDIRECT_SYMBOL_ABS) != 0 {
                continue;
            }
            slots.push((
                usize::try_from(entry).ok()?,
                Slot {
                    address: section.address.checked_add(u64::try_from(slot * 8).ok()?)?,
                    unbound: unbound.try_into().ok()?,
                },
            ));
        }
    }
    Some(slots)
}

#[cfg(test)]
mod tests;
