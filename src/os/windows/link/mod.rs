//! PE32+ executable linker (plan-47-C Phase 3).
//!
//! Binds an [`EncodedImage`] into a finished PE32+ `.exe`: builds `.idata`
//! (import directory + ILTs + IATs + hint/name table) from `image.imports`,
//! appends one `FF 25` IAT thunk per imported function to `.text`, patches every
//! relocation, emits the `.reloc` base-relocation table that makes the image
//! ASLR-capable (bug-504), and hands the laid-out sections to [`pe::write_image`].
//!
//! Mirrors `src/os/linux/link/mod.rs`: the x86 `rel32 = target − (site+4)` math
//! and the `FF 25 disp32` thunk are byte-for-byte the ELF path's, with the IAT
//! slot standing in for the GOT slot. Determinism: imports are grouped by DLL on
//! first appearance, never via `HashMap` iteration (§1 / bug-87).
//!
//! The parent `windows` module carries the `dead_code` allow (47-D removes it)
//! since nothing calls `write_executable` until the backend is wired.

mod pe;
// plan-66-K: the `.rsrc` resource section (icon + DPI manifest + version info).
mod rsrc;
// plan-66-J: the message-loop ↔ worker spike (test-only), proving the app-mode
// premise before the full Win32 floor is built.
#[cfg(test)]
mod spike;

use crate::arch::image::{EncodedImage, EncodedSection, ImportKind};
use crate::os::debug_symbols::DebugSymbolKind;
// bug-433: the shared provenance marker every in-tree linker emits. PE has no
// `PT_NOTE`/`LC_NOTE` equivalent, so it carries the same owner+descriptor bytes
// in a dedicated read-only `.mfbnote` section.
use crate::os::note::{mfb_note_descriptor, MFB_NOTE_OWNER};
use pe::{
    align_up, build_reloc, section_name, size_of_headers, ImportDirectories, Section, SCN_DATA,
    SCN_IDATA, SCN_RDATA, SCN_RELOC, SCN_TEXT,
};
use std::collections::HashMap;

const SECTION_ALIGNMENT: u32 = 0x1000;
const FILE_ALIGNMENT: u32 = 0x200;
const THUNK_SIZE: usize = 12;

/// Where every imported symbol landed: the RVA of its IAT slot (what a thunk and
/// data directory `[12]` point at) and the RVA of its `FF 25` thunk in `.text`
/// (what an external `call_pc32` relocation targets).
#[derive(Default)]
struct ImportLayout {
    iat_slot_rva: HashMap<String, u32>,
    thunk_rva: HashMap<String, u32>,
}

/// The built `.idata` blob plus the data-directory entries the optional header
/// needs to point at it.
struct IData {
    bytes: Vec<u8>,
    /// Data directory `[1]`: (import directory table RVA, its size incl. the
    /// zero terminator).
    import_dir: (u32, u32),
    /// Data directory `[12]`: (first IAT RVA, total IAT bytes).
    iat: (u32, u32),
    /// IAT slot RVA per imported symbol.
    slot_rva: HashMap<String, u32>,
}

/// Distinct import DLLs in first-seen order, each with its imported symbols in
/// image order. Grouping never depends on `HashMap` iteration (determinism).
fn group_imports_by_dll(image: &EncodedImage) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for import in &image.imports {
        match groups.iter_mut().find(|(dll, _)| *dll == import.library) {
            Some((_, symbols)) => symbols.push(import.symbol.clone()),
            None => groups.push((import.library.clone(), vec![import.symbol.clone()])),
        }
    }
    groups
}

/// Build the `.idata` section (§4.5) at `idata_rva`. Layout, in order:
/// import directory table (20 bytes/DLL + zero terminator), then per-DLL ILT,
/// per-DLL IAT (byte-identical to the ILT at emit time), then the hint/name
/// table and the DLL name strings.
fn build_idata(image: &EncodedImage, idata_rva: u32) -> IData {
    let groups = group_imports_by_dll(image);
    // Section sub-block offsets (relative to idata start).
    let dir_size = (groups.len() + 1) * 20; // +1 zero-terminator descriptor
    let ilt_size: usize = groups.iter().map(|(_, s)| (s.len() + 1) * 8).sum();
    let iat_size = ilt_size; // parallel arrays
    let dir_off = 0usize;
    let ilt_off = dir_off + dir_size;
    let iat_off = ilt_off + ilt_size;
    let hint_off = iat_off + iat_size;

    // Hint/name entries: u16 hint (0) + name + NUL, padded to an even length.
    // Compute each entry's offset within the hint/name region and the total.
    let mut hint_entry_off: HashMap<String, usize> = HashMap::new();
    let mut cursor = hint_off;
    for (_, symbols) in &groups {
        for symbol in symbols {
            hint_entry_off.insert(symbol.clone(), cursor);
            let raw = 2 + symbol.len() + 1;
            cursor += raw + (raw & 1); // pad to even
        }
    }
    // DLL name strings follow the hint/name table.
    let mut dll_name_off: HashMap<String, usize> = HashMap::new();
    for (dll, _) in &groups {
        dll_name_off.insert(dll.clone(), cursor);
        cursor += dll.len() + 1;
    }
    let total = cursor;

    let mut bytes = vec![0u8; total];
    let rva = |off: usize| idata_rva + off as u32;

    // Import directory descriptors.
    let mut slot_rva = HashMap::new();
    let mut ilt_cursor = ilt_off;
    let mut iat_cursor = iat_off;
    for (index, (dll, symbols)) in groups.iter().enumerate() {
        let desc = dir_off + index * 20;
        let ilt_rva = rva(ilt_cursor);
        let iat_rva = rva(iat_cursor);
        write_u32(&mut bytes, desc, ilt_rva); // OriginalFirstThunk (ILT)
        write_u32(&mut bytes, desc + 4, 0); // TimeDateStamp
        write_u32(&mut bytes, desc + 8, 0); // ForwarderChain
        write_u32(&mut bytes, desc + 12, rva(dll_name_off[dll])); // Name
        write_u32(&mut bytes, desc + 16, iat_rva); // FirstThunk (IAT)

        // ILT + IAT entries (identical at emit time): import-by-name = RVA of the
        // hint/name entry, bit 63 clear.
        for symbol in symbols {
            let by_name = rva(hint_entry_off[symbol]) as u64;
            write_u64(&mut bytes, ilt_cursor, by_name);
            write_u64(&mut bytes, iat_cursor, by_name);
            slot_rva.insert(symbol.clone(), rva(iat_cursor));
            ilt_cursor += 8;
            iat_cursor += 8;
        }
        // Null terminators.
        write_u64(&mut bytes, ilt_cursor, 0);
        write_u64(&mut bytes, iat_cursor, 0);
        ilt_cursor += 8;
        iat_cursor += 8;
    }
    // Directory zero terminator is already zeroed by the initial fill.

    // Hint/name entries.
    for (_, symbols) in &groups {
        for symbol in symbols {
            let off = hint_entry_off[symbol];
            // hint u16 = 0 (already zero); name + NUL.
            bytes[off + 2..off + 2 + symbol.len()].copy_from_slice(symbol.as_bytes());
            // NUL and pad byte already zero.
        }
    }
    // DLL name strings.
    for (dll, _) in &groups {
        let off = dll_name_off[dll];
        bytes[off..off + dll.len()].copy_from_slice(dll.as_bytes());
    }

    IData {
        bytes,
        import_dir: (rva(dir_off), dir_size as u32),
        iat: (rva(iat_off), iat_size as u32),
        slot_rva,
    }
}

/// Append one `FF 25 disp32` thunk per imported function to `.text`, each jumping
/// through its IAT slot. Records the thunk RVA per symbol. Byte-identical to the
/// ELF x86 PLT stub (`src/os/linux/link/mod.rs:504`).
fn append_thunks(
    text: &mut Vec<u8>,
    image: &EncodedImage,
    text_rva: u32,
    slot_rva: &HashMap<String, u32>,
    layout: &mut ImportLayout,
) -> Result<(), String> {
    for import in &image.imports {
        if import.kind != ImportKind::Function {
            continue;
        }
        let thunk_rva = text_rva + text.len() as u32;
        let slot = *slot_rva
            .get(&import.symbol)
            .ok_or_else(|| format!("windows linker: import '{}' has no IAT slot", import.symbol))?;
        // FF 25 disp32: jmp [rip + disp32]; disp32 relative to the next
        // instruction (thunk + 6). Padded to 12 bytes with int3 (0xCC).
        text.push(0xff);
        text.push(0x25);
        let rip = thunk_rva as i64 + 6;
        let delta = slot as i64 - rip;
        let disp = i32::try_from(delta).map_err(|_| {
            format!(
                "windows linker: IAT thunk displacement {delta} exceeds the ±2 GiB reach of rel32"
            )
        })?;
        text.extend_from_slice(&disp.to_le_bytes());
        text.extend_from_slice(&[0xcc; 6]);
        layout.thunk_rva.insert(import.symbol.clone(), thunk_rva);
        layout.iat_slot_rva.insert(import.symbol.clone(), slot);
    }
    Ok(())
}

/// The RVA of a defined symbol. Text symbols land at `text_rva + offset`; data
/// symbols at `data_base_rva + offset` — because `.data` is placed immediately
/// after the page-aligned `.rdata`, one base serves both partitions (§4.4).
fn symbol_rva(
    image: &EncodedImage,
    name: &str,
    text_rva: u32,
    data_base_rva: u32,
) -> Result<u32, String> {
    let symbol = image
        .symbols
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("windows linker: undefined symbol '{name}'"))?;
    Ok(match symbol.section {
        EncodedSection::Text => text_rva + symbol.offset as u32,
        EncodedSection::Data => data_base_rva + symbol.offset as u32,
    })
}

/// Patch every relocation into `.text` (§4.6). RIP-relative `rel32` at the disp32
/// field: `rel32 = target_rva − (site_rva + 4)`. External calls target the
/// symbol's `FF 25` thunk (the PE analog of a PLT stub).
fn patch_relocations(
    text: &mut [u8],
    image: &EncodedImage,
    text_rva: u32,
    data_base_rva: u32,
    layout: &ImportLayout,
) -> Result<(), String> {
    for reloc in &image.relocations {
        let site_rva = text_rva + reloc.offset as u32;
        match (reloc.binding.as_str(), reloc.kind.as_str()) {
            ("internal", "call_pc32") => {
                let target = symbol_rva(image, &reloc.target, text_rva, data_base_rva)?;
                write_rel32(text, reloc.offset, target, site_rva)?;
            }
            ("data", "data_pc32") => {
                let target = symbol_rva(image, &reloc.target, text_rva, data_base_rva)?;
                write_rel32(text, reloc.offset, target, site_rva)?;
            }
            ("external", "call_pc32") => {
                let target = *layout.thunk_rva.get(&reloc.target).ok_or_else(|| {
                    format!(
                        "windows linker cannot bind external call '{}' from {}",
                        reloc.target,
                        reloc.library.as_deref().unwrap_or("<unknown DLL>")
                    )
                })?;
                write_rel32(text, reloc.offset, target, site_rva)?;
            }
            ("external", "data_pc32") | ("external", "got_pc32") => {
                // Imported data global: the rel32 targets the IAT slot holding the
                // resolved address. The built-in surface is function-only today, so
                // this arm exists for completeness and is bound to the slot RVA.
                let target = *layout.iat_slot_rva.get(&reloc.target).ok_or_else(|| {
                    format!(
                        "windows linker cannot bind external data '{}' from {}",
                        reloc.target,
                        reloc.library.as_deref().unwrap_or("<unknown DLL>")
                    )
                })?;
                write_rel32(text, reloc.offset, target, site_rva)?;
            }
            (binding, kind) => {
                return Err(format!(
                    "windows linker does not support relocation {binding} {kind}"
                ));
            }
        }
    }
    Ok(())
}

/// Link `image` into a complete PE32+ `.exe` byte image. The entry symbol must
/// resolve to `.text`.
///
/// `gui` selects the PE subsystem: `false` → console (`WINDOWS_CUI`, 3), `true` →
/// GUI (`WINDOWS_GUI`, 2) for app-mode builds (plan-66-I). `app_icon`/`app_version`
/// carry the resources packaged into the `.rsrc` section (plan-66-K).
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_executable(
    image: &EncodedImage,
    gui: bool,
    app_icon: Option<&std::path::Path>,
    app_version: Option<&str>,
) -> Result<Vec<u8>, String> {
    // Entry must be a text symbol (§4.6, mirroring the ELF requirement).
    let entry_offset = image
        .symbols
        .iter()
        .find(|s| s.name == image.entry && s.section == EncodedSection::Text)
        .map(|s| s.offset)
        .ok_or_else(|| format!("entry symbol '{}' does not resolve to text", image.entry))?;

    // Sections present (zero-length sections are omitted — §4.4).
    let rodata_size = image.rodata_size.min(image.data.len());
    let has_rdata = rodata_size > 0;
    let has_data = image.data.len() > rodata_size;
    let has_idata = !image.imports.is_empty();
    // plan-66-K: app-mode (GUI) builds carry a `.rsrc` (DPI manifest, always; icon
    // and version when supplied). Console builds get no `.rsrc`, so they stay
    // byte-identical to the pre-K writer.
    let has_rsrc = gui;
    // bug-432: a signed build carries the `mfb-signing-v1` blob verbatim in a
    // read-only `.mfbsign` section (the 8-char name unified with ELF/Mach-O). It is
    // placed last (after `.rsrc`), so its size shifts no other section's RVA, and
    // gets no data directory — this is MFBASIC's own provenance blob, not
    // Authenticode. Unsigned builds omit it and stay byte-identical to before.
    let has_sign = image.signing_metadata.is_some();

    // Count sections to size the headers, then lay out RVAs/file offsets. The
    // `.reloc` base-relocation section (bug-504) and the `.mfbnote` provenance
    // section (bug-433) are unconditional — the two trailing +1s; the `.mfbsign`
    // signing section (bug-432) is present only on a signed build.
    let section_count = 1
        + has_rdata as usize
        + has_data as usize
        + has_idata as usize
        + has_rsrc as usize
        + 1
        + 1
        + has_sign as usize;
    let headers = size_of_headers(section_count);
    let text_rva = align_up(headers, SECTION_ALIGNMENT);
    let text_file = align_up(headers, FILE_ALIGNMENT);

    // .text = image.text + one 12-byte thunk per function import. Build the
    // thunks first so .text's size (and every later RVA) is final; that needs the
    // IAT slot RVAs, which need .idata's RVA, which needs .text's final size — so
    // compute .text's final length from the thunk count first, then place .idata.
    let function_imports = image
        .imports
        .iter()
        .filter(|i| i.kind == ImportKind::Function)
        .count();
    let text_len_final = image.text.len() + function_imports * THUNK_SIZE;

    // RVA/file progression for the data-bearing sections after .text.
    let mut next_rva = align_up(text_rva + text_len_final as u32, SECTION_ALIGNMENT);
    let mut next_file = align_up(text_file + text_len_final as u32, FILE_ALIGNMENT);

    let rdata_rva = next_rva;
    let rdata_file = next_file;
    if has_rdata {
        next_rva = align_up(next_rva + rodata_size as u32, SECTION_ALIGNMENT);
        next_file = align_up(next_file + rodata_size as u32, FILE_ALIGNMENT);
    }
    let data_rva = next_rva;
    let data_file = next_file;
    let data_len = image.data.len() - rodata_size;
    if has_data {
        next_rva = align_up(next_rva + data_len as u32, SECTION_ALIGNMENT);
        next_file = align_up(next_file + data_len as u32, FILE_ALIGNMENT);
    }
    let idata_rva = next_rva;
    let idata_file = next_file;

    // data_base_rva: RVA of image.data[0]. .data sits at rdata_rva + rodata_size
    // (== data_rva, since rodata_size is page-aligned by construction), so one
    // base serves both partitions (§4.4). When there is no .rdata, the base is
    // wherever .data starts.
    let data_base_rva = if has_rdata { rdata_rva } else { data_rva };
    if has_rdata && has_data {
        assert_eq!(
            rdata_rva + rodata_size as u32,
            data_rva,
            "the .rdata/.data split must keep data symbol RVAs contiguous (§4.4)"
        );
    }

    // Build .idata and the thunks.
    let mut text = image.text.clone();
    let mut layout = ImportLayout::default();
    let idata = if has_idata {
        let idata = build_idata(image, idata_rva);
        append_thunks(&mut text, image, text_rva, &idata.slot_rva, &mut layout)?;
        Some(idata)
    } else {
        None
    };
    assert_eq!(
        text.len(),
        text_len_final,
        "final .text length must match the reserved layout"
    );

    // Patch relocations now that every target RVA is known.
    patch_relocations(&mut text, image, text_rva, data_base_rva, &layout)?;

    // Assemble the section list (in file order).
    let rdata_bytes = &image.data[..rodata_size];
    let data_bytes = &image.data[rodata_size..];
    let empty: [u8; 0] = [];
    let idata_bytes: &[u8] = idata.as_ref().map(|i| i.bytes.as_slice()).unwrap_or(&empty);

    // plan-66-K: the `.rsrc` section sits last (its size affects no later RVA). Its
    // RVA/file follow `.idata` (or `.idata`'s reserved slot when there are no
    // imports). Built before the section list so its bytes outlive the borrow;
    // `build_rsrc` needs the RVA because a resource data entry's `OffsetToData` is
    // an image RVA.
    let (rsrc_rva, rsrc_file) = if has_idata {
        (
            align_up(idata_rva + idata_bytes.len() as u32, SECTION_ALIGNMENT),
            align_up(idata_file + idata_bytes.len() as u32, FILE_ALIGNMENT),
        )
    } else {
        (idata_rva, idata_file)
    };
    let rsrc_bytes: Vec<u8> = if has_rsrc {
        rsrc::build_rsrc(rsrc_rva, app_icon, app_version)?
    } else {
        Vec::new()
    };

    // bug-504: the `.reloc` base-relocation table follows `.rsrc` (link.exe's
    // order), or takes its reserved slot when there is no `.rsrc`. The linker
    // patches only RIP-relative references, so there are no DIR64 fixups to list
    // (measured: the fixture corpus linked at two different image bases differs in
    // no section byte) — the body is `build_reloc`'s one padding block, which is
    // what lets the loader slide a `DYNAMIC_BASE` image whose `RELOCS_STRIPPED`
    // bit is clear.
    let (reloc_rva, reloc_file) = if has_rsrc {
        (
            align_up(rsrc_rva + rsrc_bytes.len() as u32, SECTION_ALIGNMENT),
            align_up(rsrc_file + rsrc_bytes.len() as u32, FILE_ALIGNMENT),
        )
    } else {
        (rsrc_rva, rsrc_file)
    };
    let reloc_bytes = build_reloc(&[], text_rva);

    // bug-433: the unconditional `.mfbnote` provenance section follows `.reloc`,
    // so its size affects no earlier RVA and it carries no data-directory entry.
    // Body is the same owner+descriptor framing the ELF `PT_NOTE`/Mach-O `LC_NOTE`
    // use, so a reader can locate `MFBasic\0` and read the 16-byte descriptor in
    // any of the three formats.
    let (mfbnote_rva, mfbnote_file) = (
        align_up(reloc_rva + reloc_bytes.len() as u32, SECTION_ALIGNMENT),
        align_up(reloc_file + reloc_bytes.len() as u32, FILE_ALIGNMENT),
    );
    let mut mfbnote_bytes = Vec::with_capacity(MFB_NOTE_OWNER.len() + mfb_note_descriptor().len());
    mfbnote_bytes.extend_from_slice(MFB_NOTE_OWNER);
    mfbnote_bytes.extend_from_slice(&mfb_note_descriptor());

    let mut sections = vec![Section {
        name: section_name(".text"),
        characteristics: SCN_TEXT,
        virtual_address: text_rva,
        virtual_size: text.len() as u32,
        file_offset: text_file,
        bytes: &text,
    }];
    if has_rdata {
        sections.push(Section {
            name: section_name(".rdata"),
            characteristics: SCN_RDATA,
            virtual_address: rdata_rva,
            virtual_size: rodata_size as u32,
            file_offset: rdata_file,
            bytes: rdata_bytes,
        });
    }
    if has_data {
        sections.push(Section {
            name: section_name(".data"),
            characteristics: SCN_DATA,
            virtual_address: data_rva,
            virtual_size: data_len as u32,
            file_offset: data_file,
            bytes: data_bytes,
        });
    }
    if has_idata {
        sections.push(Section {
            name: section_name(".idata"),
            characteristics: SCN_IDATA,
            virtual_address: idata_rva,
            virtual_size: idata_bytes.len() as u32,
            file_offset: idata_file,
            bytes: idata_bytes,
        });
    }

    let mut dirs = idata
        .as_ref()
        .map(|i| ImportDirectories {
            import: i.import_dir,
            iat: i.iat,
            ..ImportDirectories::default()
        })
        .unwrap_or_default();
    if has_rsrc {
        dirs.resource = (rsrc_rva, rsrc_bytes.len() as u32);
        sections.push(Section {
            name: section_name(".rsrc"),
            characteristics: SCN_RDATA, // initialized data, read-only
            virtual_address: rsrc_rva,
            virtual_size: rsrc_bytes.len() as u32,
            file_offset: rsrc_file,
            bytes: &rsrc_bytes,
        });
    }
    // bug-504: `.reloc` + data directory [5] BASERELOC pointing at it.
    dirs.basereloc = (reloc_rva, reloc_bytes.len() as u32);
    sections.push(Section {
        name: section_name(".reloc"),
        characteristics: SCN_RELOC,
        virtual_address: reloc_rva,
        virtual_size: reloc_bytes.len() as u32,
        file_offset: reloc_file,
        bytes: &reloc_bytes,
    });
    // bug-433: `.mfbnote` — unconditional, no data directory. `.mfbnote` is exactly
    // 8 chars, fitting PE's 8-byte section-name field with no truncation.
    sections.push(Section {
        name: section_name(".mfbnote"),
        characteristics: SCN_RDATA, // initialized data, read-only
        virtual_address: mfbnote_rva,
        virtual_size: mfbnote_bytes.len() as u32,
        file_offset: mfbnote_file,
        bytes: &mfbnote_bytes,
    });

    // bug-432: the `.mfbsign` section goes last — after the unconditional
    // `.mfbnote`, so the two trailing sections never overlap — so its size shifts no
    // earlier RVA. It carries the blob verbatim, read-only, with no data directory.
    if let Some(metadata) = image
        .signing_metadata
        .as_ref()
        .map(|signing| signing.metadata.as_slice())
    {
        let sign_rva = align_up(mfbnote_rva + mfbnote_bytes.len() as u32, SECTION_ALIGNMENT);
        let sign_file = align_up(mfbnote_file + mfbnote_bytes.len() as u32, FILE_ALIGNMENT);
        sections.push(Section {
            name: section_name(".mfbsign"),
            characteristics: SCN_RDATA, // initialized data, read-only
            virtual_address: sign_rva,
            virtual_size: metadata.len() as u32,
            file_offset: sign_file,
            bytes: metadata,
        });
    }

    // plan-166: a `--debug` build lists its symbols in a COFF symbol table.
    let coff_symbols = if image.debug_symbols {
        Some(coff_symbol_table(image, rodata_size, has_rdata, has_data)?)
    } else {
        None
    };
    let mut bytes = pe::write_image(
        &sections,
        text_rva + entry_offset as u32,
        dirs,
        gui,
        coff_symbols.as_ref(),
    );
    // The image is complete: seal the content signature over it.
    if let Some(signing) = &image.signing_metadata {
        crate::os::content_signature::seal(&mut bytes, signing)?;
    }
    Ok(bytes)
}

/// `IMAGE_SYM_CLASS_EXTERNAL`: what MinGW emits for a non-static function, and
/// what `llvm-nm` shows as `T` / `D`.
const IMAGE_SYM_CLASS_EXTERNAL: u8 = 2;
/// `IMAGE_SYM_DTYPE_FUNCTION << 4`: the `Type` of a function symbol.
const IMAGE_SYM_TYPE_FUNCTION: u16 = 0x20;

/// The COFF symbol table a `--debug` `.exe` carries (plan-166): one 18-byte
/// record per debug symbol, in the shared order. `Value` is the offset within
/// the record's section (measured: `llvm-nm` adds `ImageBase` + the section RVA).
/// `SectionNumber` is the 1-based index in the emitted order, which always opens
/// `.text`, then `.rdata` and `.data` when present.
fn coff_symbol_table(
    image: &EncodedImage,
    rodata_size: usize,
    has_rdata: bool,
    has_data: bool,
) -> Result<pe::CoffSymbolTable, String> {
    let rdata_number = 1 + has_rdata as i16;
    let data_number = rdata_number + has_data as i16;
    let symbols = crate::os::debug_symbols::debug_symbols(image)?;
    let mut records = Vec::with_capacity(symbols.len() * 18);
    let mut names = Vec::new();
    for symbol in &symbols {
        let (section_number, value, type_) = match symbol.kind {
            DebugSymbolKind::Function => (1, symbol.offset, IMAGE_SYM_TYPE_FUNCTION),
            DebugSymbolKind::Object if symbol.offset < rodata_size => {
                (rdata_number, symbol.offset, 0)
            }
            DebugSymbolKind::Object if has_data => (data_number, symbol.offset - rodata_size, 0),
            DebugSymbolKind::Object => {
                return Err(format!(
                    "debug symbols: data symbol '{}' at offset {} lands in no section",
                    symbol.name, symbol.offset
                ))
            }
        };
        let name = symbol.name.as_bytes();
        if name.len() <= 8 {
            let mut inline = [0u8; 8];
            inline[..name.len()].copy_from_slice(name);
            records.extend_from_slice(&inline);
        } else {
            // A long name: zeroes, then its offset into the string table, which
            // counts the table's own 4-byte length field.
            records.extend_from_slice(&0u32.to_le_bytes());
            records.extend_from_slice(&(4 + names.len() as u32).to_le_bytes());
            names.extend_from_slice(name);
            names.push(0);
        }
        records.extend_from_slice(&(value as u32).to_le_bytes());
        records.extend_from_slice(&section_number.to_le_bytes());
        records.extend_from_slice(&type_.to_le_bytes());
        records.push(IMAGE_SYM_CLASS_EXTERNAL);
        records.push(0); // NumberOfAuxSymbols
    }
    let mut strings = (4 + names.len() as u32).to_le_bytes().to_vec();
    strings.extend_from_slice(&names);
    Ok(pe::CoffSymbolTable {
        records,
        strings,
        count: symbols.len() as u32,
    })
}

fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// Patch a RIP-relative `rel32` at `offset` (the disp32 field): the next
/// instruction is `site_rva + 4`, so `rel32 = target_rva − (site_rva + 4)`.
fn write_rel32(
    text: &mut [u8],
    offset: usize,
    target_rva: u32,
    site_rva: u32,
) -> Result<(), String> {
    if offset + 4 > text.len() {
        return Err(format!(
            "windows linker: relocation offset {offset} out of range"
        ));
    }
    let rel = target_rva as i64 - (site_rva as i64 + 4);
    let rel = i32::try_from(rel)
        .map_err(|_| format!("windows linker: rel32 displacement {rel} exceeds ±2 GiB"))?;
    text[offset..offset + 4].copy_from_slice(&rel.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests;
