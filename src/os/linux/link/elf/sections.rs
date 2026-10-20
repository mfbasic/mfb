//! plan-176-O: the ELF writer's section table — section records, the symbol table and appending
//! the sections. The static/dynamic image encoders and the header helpers live in
//! `linux/link/elf/mod.rs`.

use super::*;

/// One section header beyond the leading `SHT_NULL` entry and the trailing
/// `.shstrtab`, which [`append_section_table`] adds itself.
pub(super) struct ElfSection<'a> {
    pub(super) name: &'static str,
    pub(super) type_: u32,
    pub(super) flags: u64,
    pub(super) addr: u64,
    pub(super) contents: ElfSectionContents<'a>,
    pub(super) link: u32,
    pub(super) info: u32,
    pub(super) addralign: u64,
    pub(super) entsize: u64,
}

/// Where a section's bytes are.
pub(super) enum ElfSectionContents<'a> {
    /// Bytes a loaded segment already placed in the file.
    InFile { offset: usize, size: usize },
    /// Bytes appended past the current file end, at a `file_align` boundary.
    Appended { bytes: &'a [u8], file_align: usize },
}

/// Append `sections` and a section header table describing them, and point the
/// ELF header at it: `[NULL, sections…, .shstrtab]`, with `e_shstrndx` naming
/// `.shstrtab`. Appended bodies go first (in order), then `.shstrtab`, then the
/// 8-aligned header table. Nothing the loader reads moves: section headers are
/// file-only metadata past the last `PT_LOAD`.
///
/// One builder owns the whole table because an ELF has exactly one: a signed
/// `--debug` build's `.mfbsign` and `.symtab` must share it, or the second table
/// written would orphan the first.
pub(super) fn append_section_table(bytes: &mut Vec<u8>, sections: &[ElfSection<'_>]) {
    const SHDR_SIZE: usize = 64;
    let mut shstrtab = vec![0u8];
    let mut name_offsets = Vec::with_capacity(sections.len());
    for section in sections {
        name_offsets.push(shstrtab.len() as u32);
        shstrtab.extend_from_slice(section.name.as_bytes());
        shstrtab.push(0);
    }
    let shstrtab_name = shstrtab.len() as u32;
    shstrtab.extend_from_slice(b".shstrtab\0");

    let mut placed = Vec::with_capacity(sections.len());
    for section in sections {
        placed.push(match section.contents {
            ElfSectionContents::InFile { offset, size } => (offset, size),
            ElfSectionContents::Appended {
                bytes: body,
                file_align,
            } => {
                let offset = align(bytes.len(), file_align);
                bytes.resize(offset, 0);
                bytes.extend_from_slice(body);
                (offset, body.len())
            }
        });
    }
    let shstrtab_offset = bytes.len();
    bytes.extend_from_slice(&shstrtab);
    let shoff = align(bytes.len(), 8);
    bytes.resize(shoff, 0);

    bytes.resize(bytes.len() + SHDR_SIZE, 0);
    for ((section, (offset, size)), name) in sections.iter().zip(placed).zip(name_offsets) {
        section_header(
            bytes,
            name,
            section.type_,
            section.flags,
            section.addr,
            offset as u64,
            size as u64,
            section.link,
            section.info,
            section.addralign,
            section.entsize,
        );
    }
    section_header(
        bytes,
        shstrtab_name,
        SHT_STRTAB,
        0,
        0,
        shstrtab_offset as u64,
        shstrtab.len() as u64,
        0,
        0,
        1,
        0,
    );

    let shnum = sections.len() as u16 + 2;
    bytes[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
    bytes[58..60].copy_from_slice(&(SHDR_SIZE as u16).to_le_bytes());
    bytes[60..62].copy_from_slice(&shnum.to_le_bytes());
    bytes[62..64].copy_from_slice(&(shnum - 1).to_le_bytes());
}

const SHT_PROGBITS: u32 = 1;

const SHT_STRTAB: u32 = 3;

/// The `.mfbsign` section a signed build carries (bug-432 named it the same on
/// every format): the blob, appended 8-aligned, `sh_addralign` 1.
fn mfbsign_section(metadata: &[u8]) -> ElfSection<'_> {
    ElfSection {
        name: ".mfbsign",
        type_: SHT_PROGBITS,
        flags: 0,
        addr: 0,
        contents: ElfSectionContents::Appended {
            bytes: metadata,
            file_align: 8,
        },
        link: 0,
        info: 0,
        addralign: 1,
        entsize: 0,
    }
}

/// Where a writer placed its loaded bytes, for the `--debug` section headers.
pub(super) struct LoadedSections {
    pub(super) text_vmaddr: u64,
    /// Including the PLT stubs appended after `image.text`.
    pub(super) text_len: usize,
    pub(super) data_offset: usize,
    pub(super) data_vmaddr: u64,
    pub(super) data_len: usize,
    /// The read-only prefix of data the dynamic writer maps as its own `PT_LOAD`
    /// (and so its own `.rodata` section); 0 for the static writers, which map
    /// all data writable.
    pub(super) rodata_size: usize,
    /// The dynamic writer's `.dynstr` and `.dynamic`; `None` for a static image.
    pub(super) dynamic: Option<DynamicSections>,
}

/// File offsets, addresses and sizes of the loader's `.dynstr` and `.dynamic`.
pub(super) struct DynamicSections {
    pub(super) dynstr_offset: usize,
    pub(super) dynstr_vmaddr: u64,
    pub(super) dynstr_size: usize,
    pub(super) dynamic_offset: usize,
    pub(super) dynamic_vmaddr: u64,
    pub(super) dynamic_size: usize,
}

const SHT_DYNAMIC: u32 = 6;

const SHT_SYMTAB: u32 = 2;

const SHF_WRITE: u64 = 1;

const SHF_ALLOC: u64 = 2;

const SHF_EXECINSTR: u64 = 4;

const STB_LOCAL: u8 = 0;

const STT_OBJECT: u8 = 1;

const STT_FUNC: u8 = 2;

const SYMTAB_ENTRY_SIZE: usize = 24;

/// The section header table a writer ends with, if any:
///
/// - no `--debug`, unsigned: none (no table at all);
/// - no `--debug`, signed: `[NULL, (.dynstr, .dynamic), .mfbsign, .shstrtab]`;
/// - `--debug`: `[NULL, .text, (.rodata), .data, (.dynstr, .dynamic), .symtab,
///   .strtab, (.mfbsign), .shstrtab]`, every symbol `STB_LOCAL`.
///
/// A dynamic image's table always carries `.dynamic` (linked to `.dynstr`):
/// once an ELF has section headers, readers look for the section that covers
/// `PT_DYNAMIC`, and GNU readelf reports `no .dynamic section in the dynamic
/// segment` without it.
pub(super) fn append_elf_sections(
    bytes: &mut Vec<u8>,
    loaded: &LoadedSections,
    debug: Option<&[DebugSymbol<'_>]>,
    signing_metadata: Option<&[u8]>,
) {
    if debug.is_none() && signing_metadata.is_none() {
        return;
    }
    let rodata_size = loaded.rodata_size.min(loaded.data_len);
    // Section indices (1-based; 0 is the NULL entry).
    let loaded_count = if debug.is_some() {
        2 + (rodata_size > 0) as u16
    } else {
        0
    };
    let text_index = 1u16;
    let rodata_index = (rodata_size > 0).then_some(2u16);
    let data_index = text_index + 1 + rodata_index.is_some() as u16;
    let dynstr_index = loaded_count + 1;
    let dynamic_sections = loaded.dynamic.as_ref().map(|dynamic| {
        [
            ElfSection {
                name: ".dynstr",
                type_: SHT_STRTAB,
                flags: SHF_ALLOC,
                addr: dynamic.dynstr_vmaddr,
                contents: ElfSectionContents::InFile {
                    offset: dynamic.dynstr_offset,
                    size: dynamic.dynstr_size,
                },
                link: 0,
                info: 0,
                addralign: 1,
                entsize: 0,
            },
            ElfSection {
                name: ".dynamic",
                type_: SHT_DYNAMIC,
                flags: SHF_ALLOC | SHF_WRITE,
                addr: dynamic.dynamic_vmaddr,
                contents: ElfSectionContents::InFile {
                    offset: dynamic.dynamic_offset,
                    size: dynamic.dynamic_size,
                },
                link: dynstr_index as u32,
                info: 0,
                addralign: 8,
                entsize: 16,
            },
        ]
    });
    let Some(debug) = debug else {
        let mut sections: Vec<_> = dynamic_sections.into_iter().flatten().collect();
        sections.extend(signing_metadata.map(mfbsign_section));
        append_section_table(bytes, &sections);
        return;
    };
    let symtab_index = loaded_count + 1 + 2 * dynamic_sections.is_some() as u16;
    let strtab_index = symtab_index + 1;
    let (symtab, strtab) = symtab_bytes(
        debug,
        loaded,
        text_index,
        rodata_index.map(|index| (index, rodata_size)),
        data_index,
    );
    let loaded_section = |name, flags, addr, offset, size| ElfSection {
        name,
        type_: SHT_PROGBITS,
        flags,
        addr,
        contents: ElfSectionContents::InFile { offset, size },
        link: 0,
        info: 0,
        addralign: 16,
        entsize: 0,
    };
    let mut sections = vec![loaded_section(
        ".text",
        SHF_ALLOC | SHF_EXECINSTR,
        loaded.text_vmaddr,
        TEXT_FILE_OFFSET,
        loaded.text_len,
    )];
    if rodata_index.is_some() {
        sections.push(loaded_section(
            ".rodata",
            SHF_ALLOC,
            loaded.data_vmaddr,
            loaded.data_offset,
            rodata_size,
        ));
    }
    sections.push(loaded_section(
        ".data",
        SHF_ALLOC | SHF_WRITE,
        loaded.data_vmaddr + rodata_size as u64,
        loaded.data_offset + rodata_size,
        loaded.data_len - rodata_size,
    ));
    sections.extend(dynamic_sections.into_iter().flatten());
    sections.push(ElfSection {
        name: ".symtab",
        type_: SHT_SYMTAB,
        flags: 0,
        addr: 0,
        contents: ElfSectionContents::Appended {
            bytes: &symtab,
            file_align: 8,
        },
        link: strtab_index as u32,
        // One past the last local: every entry (the NULL one included) is local.
        info: (1 + debug.len()) as u32,
        addralign: 8,
        entsize: SYMTAB_ENTRY_SIZE as u64,
    });
    sections.push(ElfSection {
        name: ".strtab",
        type_: SHT_STRTAB,
        flags: 0,
        addr: 0,
        contents: ElfSectionContents::Appended {
            bytes: &strtab,
            file_align: 1,
        },
        link: 0,
        info: 0,
        addralign: 1,
        entsize: 0,
    });
    if let Some(metadata) = signing_metadata {
        sections.push(mfbsign_section(metadata));
    }
    append_section_table(bytes, &sections);
}

/// The `.symtab` entries (a NULL entry, then one `STB_LOCAL` entry per debug
/// symbol) and the `.strtab` they name. Addresses use the relocation rule with no
/// read-only window, as `patch_relocations` does on Linux: a data symbol is
/// `data_vmaddr + offset` whether or not the dynamic writer maps its prefix
/// read-only.
fn symtab_bytes(
    symbols: &[DebugSymbol<'_>],
    loaded: &LoadedSections,
    text_index: u16,
    rodata: Option<(u16, usize)>,
    data_index: u16,
) -> (Vec<u8>, Vec<u8>) {
    let mut symtab = vec![0u8; SYMTAB_ENTRY_SIZE];
    let mut strtab = vec![0u8];
    for symbol in symbols {
        let (kind, shndx) = match symbol.kind {
            DebugSymbolKind::Function => (STT_FUNC, text_index),
            DebugSymbolKind::Object => match rodata {
                Some((index, size)) if symbol.offset < size => (STT_OBJECT, index),
                _ => (STT_OBJECT, data_index),
            },
        };
        put_u32(&mut symtab, strtab.len() as u32);
        symtab.push((STB_LOCAL << 4) | kind);
        symtab.push(0);
        put_u16(&mut symtab, shndx);
        put_u64(
            &mut symtab,
            section_vmaddr(
                symbol.section,
                symbol.offset,
                loaded.text_vmaddr,
                loaded.data_vmaddr,
                None,
            ),
        );
        put_u64(&mut symtab, symbol.size as u64);
        strtab.extend_from_slice(symbol.name.as_bytes());
        strtab.push(0);
    }
    (symtab, strtab)
}
