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
        debug_assert_eq!(
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
    debug_assert_eq!(
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
    if let Some(metadata) = image.signing_metadata.as_deref() {
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

    Ok(pe::write_image(
        &sections,
        text_rva + entry_offset as u32,
        dirs,
        gui,
    ))
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
mod tests {
    use super::*;
    use crate::arch::image::{EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind};

    fn image(text: Vec<u8>) -> EncodedImage {
        EncodedImage {
            text,
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![EncodedSymbol {
                name: "_start".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_start".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
        }
    }

    /// A genuinely *runnable* `ExitProcess(42)` console program (the plan's 47-C
    /// runtime proof). Unlike `exit_process_42_image` (a structural fixture), the
    /// entry is Win64-ABI-correct: it reserves the 32-byte shadow space (+8 for
    /// 16-alignment at the call) before invoking `kernel32!ExitProcess`.
    ///
    ///   _start:
    ///     48 83 EC 28        sub  rsp, 0x28        ; 32 shadow + 8 align
    ///     B9 2A 00 00 00     mov  ecx, 42          ; uExitCode (arg0 in ecx)
    ///     E8 <rel32>         call ExitProcess      ; via the FF25 IAT thunk
    ///     CC                 int3                  ; never reached
    fn runnable_exit42_image() -> EncodedImage {
        let mut text = vec![0x48, 0x83, 0xEC, 0x28, 0xB9, 0x2A, 0x00, 0x00, 0x00, 0xE8];
        text.extend_from_slice(&[0, 0, 0, 0]); // rel32 (disp field at offset 10)
        text.push(0xCC);
        EncodedImage {
            text,
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![EncodedSymbol {
                name: "_start".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: vec![EncodedRelocation {
                offset: 10,
                target: "ExitProcess".to_string(),
                kind: "call_pc32".to_string(),
                binding: "external".to_string(),
                library: Some("kernel32.dll".to_string()),
            }],
            imports: vec![EncodedImport {
                library: "kernel32.dll".to_string(),
                symbol: "ExitProcess".to_string(),
                kind: ImportKind::Function,
                version: None,
            }],
            entry: "_start".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
        }
    }

    /// Dev harness (not a CI assertion): when `MFB_EXIT42_OUT` is set, write the
    /// runnable `ExitProcess(42)` `.exe` there so it can be run on a real Windows
    /// host (the 47-C runtime proof). A no-op otherwise, so `cargo test` is
    /// unaffected.
    #[test]
    fn writes_runnable_exit42_exe_when_env_set() {
        let Ok(path) = std::env::var("MFB_EXIT42_OUT") else {
            return;
        };
        let bytes =
            write_executable(&runnable_exit42_image(), false, None, None).expect("link exit42");
        std::fs::write(&path, &bytes).expect("write exit42.exe");
        eprintln!("wrote {} bytes to {path}", bytes.len());
    }

    fn le_u16(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn le_u32(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }
    fn le_u64(b: &[u8], o: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[o..o + 8]);
        u64::from_le_bytes(a)
    }
    /// Scan the section table for a section whose 8-byte name field equals
    /// `name` (NUL-padded), returning that section's raw file body if present.
    fn section_named(image: &[u8], name: &[u8]) -> Option<Vec<u8>> {
        let e_lfanew = le_u32(image, 0x3C) as usize;
        let n = le_u16(image, e_lfanew + 6) as usize;
        let sect_table = e_lfanew + 4 + 20 + 240;
        let mut field = [0u8; 8];
        field[..name.len().min(8)].copy_from_slice(&name[..name.len().min(8)]);
        for i in 0..n {
            let s = sect_table + i * 40;
            if image[s..s + 8] == field {
                let raw_size = le_u32(image, s + 16) as usize;
                let raw_ptr = le_u32(image, s + 20) as usize;
                return Some(image[raw_ptr..raw_ptr + raw_size].to_vec());
            }
        }
        None
    }

    /// Read the bytes at an RVA out of a written image, by scanning the section
    /// table for the section that contains it.
    fn read_at_rva(image: &[u8], rva: u32, len: usize) -> Vec<u8> {
        let e_lfanew = le_u32(image, 0x3C) as usize;
        let n = le_u16(image, e_lfanew + 6) as usize;
        let sect_table = e_lfanew + 4 + 20 + 240;
        for i in 0..n {
            let s = sect_table + i * 40;
            let vaddr = le_u32(image, s + 12);
            let vsize = le_u32(image, s + 8);
            let raw_ptr = le_u32(image, s + 20);
            if rva >= vaddr && rva < vaddr + vsize {
                let file_off = raw_ptr + (rva - vaddr);
                return image[file_off as usize..file_off as usize + len].to_vec();
            }
        }
        panic!("rva {rva:#x} not in any section");
    }

    /// bug-504 (audit-3 LNK-13): the emitted PE must be ASLR-capable. Three header
    /// facts have to hold together — the loader ignores `DYNAMIC_BASE` when
    /// `IMAGE_FILE_RELOCS_STRIPPED` is set, and without a base-relocation directory
    /// it cannot prove the image slides — so this asserts all of them on a real
    /// linked image (imports + data), not a hand-laid header:
    /// `RELOCS_STRIPPED` clear; `DYNAMIC_BASE | HIGH_ENTROPY_VA | NX_COMPAT` set;
    /// data directory `[5]` points at a `.reloc` section whose block(s) parse.
    #[test]
    fn emitted_pe_is_aslr_capable() {
        for gui in [false, true] {
            let bytes = write_executable(&runnable_exit42_image(), gui, None, None)
                .expect("link exit42");
            let e_lfanew = le_u32(&bytes, 0x3C) as usize;
            let coff = e_lfanew + 4;
            let opt = coff + 20;
            let characteristics = le_u16(&bytes, coff + 18);
            assert_eq!(
                characteristics & 0x0001,
                0,
                "IMAGE_FILE_RELOCS_STRIPPED must be clear (gui={gui}): {characteristics:#06x}"
            );
            assert_ne!(
                characteristics & 0x0020,
                0,
                "LARGE_ADDRESS_AWARE stays set (gui={gui})"
            );
            let dll = le_u16(&bytes, opt + 70);
            assert_eq!(
                dll & (0x0040 | 0x0020 | 0x0100),
                0x0040 | 0x0020 | 0x0100,
                "DllCharacteristics needs DYNAMIC_BASE|HIGH_ENTROPY_VA|NX_COMPAT (gui={gui}): {dll:#06x}"
            );
            // Data directory [5] = BASERELOC, 112 bytes into the optional header.
            let dd = opt + 112 + 5 * 8;
            let (reloc_rva, reloc_size) = (le_u32(&bytes, dd), le_u32(&bytes, dd + 4));
            assert_ne!(reloc_size, 0, "BASERELOC directory must be populated (gui={gui})");
            let reloc = section_named(&bytes, b".reloc").expect(".reloc section present");
            // The directory points at the section start and covers exactly its body.
            let n = le_u16(&bytes, e_lfanew + 6) as usize;
            let sect_table = opt + 240;
            let reloc_hdr = (0..n)
                .map(|i| sect_table + i * 40)
                .find(|&s| &bytes[s..s + 6] == b".reloc")
                .expect(".reloc header");
            assert_eq!(le_u32(&bytes, reloc_hdr + 12), reloc_rva, "[5].RVA = .reloc RVA");
            assert_eq!(le_u32(&bytes, reloc_hdr + 8), reloc_size, "[5].Size = .reloc VirtualSize");
            assert_eq!(
                le_u32(&bytes, reloc_hdr + 36),
                0x4200_0040,
                ".reloc is INITIALIZED_DATA | DISCARDABLE | READ"
            );
            // Walk the IMAGE_BASE_RELOCATION blocks: each names a page inside the
            // image and its entries are either padding (ABSOLUTE) or DIR64.
            let size_of_image = le_u32(&bytes, opt + 56);
            let mut off = 0usize;
            let mut blocks = 0;
            while off < reloc_size as usize {
                let page = le_u32(&reloc, off);
                let block_size = le_u32(&reloc, off + 4) as usize;
                assert!(block_size >= 8 && block_size % 4 == 0, "SizeOfBlock {block_size}");
                assert_eq!(page % 0x1000, 0, "block page {page:#x} is page-aligned");
                assert!(page < size_of_image, "block page {page:#x} inside the image");
                for entry in (8..block_size).step_by(2) {
                    let kind = le_u16(&reloc, off + entry) >> 12;
                    assert!(kind == 0 || kind == 10, "entry type {kind} (ABSOLUTE or DIR64)");
                }
                off += block_size;
                blocks += 1;
            }
            assert_eq!(off, reloc_size as usize, "blocks tile the directory exactly");
            assert!(blocks >= 1, "at least one relocation block");
        }
    }

    /// bug-432: a signed Windows build emits the `mfb-signing-v1` blob verbatim in
    /// an `.mfbsign` PE section (the unified 8-char name shared with ELF/Mach-O).
    #[test]
    fn signing_metadata_emits_mfbsign_section() {
        let mut img = image(vec![0xc3]); // ret
        img.signing_metadata = Some(b"{\"format\":\"mfb-signing-v1\"}".to_vec());
        let bytes = write_executable(&img, false, None, None).expect("link");
        let section = section_named(&bytes, b".mfbsign").expect(".mfbsign section present");
        // The section body carries the blob verbatim (no PE-specific header). The
        // raw body may be FILE_ALIGNMENT-padded, so scan for the blob within it.
        assert!(
            section
                .windows(img.signing_metadata.as_ref().unwrap().len())
                .any(|w| w == img.signing_metadata.as_ref().unwrap().as_slice()),
            "the .mfbsign section body carries the blob verbatim"
        );
    }

    /// bug-432 non-goal guard: an unsigned build (the common case) emits no
    /// `.mfbsign` section (though it still carries the unconditional `.mfbnote`).
    #[test]
    fn unsigned_build_emits_no_mfbsign_section() {
        let img = image(vec![0xc3]); // signing_metadata == None
        let bytes = write_executable(&img, false, None, None).expect("link");
        assert!(section_named(&bytes, b".mfbsign").is_none());
    }

    /// The virtual `(address, size)` extent of a named section, if present.
    fn section_extent(image: &[u8], name: &[u8]) -> Option<(u32, u32)> {
        let e_lfanew = le_u32(image, 0x3C) as usize;
        let n = le_u16(image, e_lfanew + 6) as usize;
        let sect_table = e_lfanew + 4 + 20 + 240;
        let mut field = [0u8; 8];
        field[..name.len().min(8)].copy_from_slice(&name[..name.len().min(8)]);
        for i in 0..n {
            let s = sect_table + i * 40;
            if image[s..s + 8] == field {
                return Some((le_u32(image, s + 12), le_u32(image, s + 8)));
            }
        }
        None
    }

    /// bug-432 ∩ bug-433: a signed build carries BOTH trailing sections — the
    /// unconditional `.mfbnote` provenance marker and the `.mfbsign` signing blob —
    /// and their virtual ranges must not overlap (the merge hazard: both were once
    /// placed in the same post-`.rsrc` slot).
    #[test]
    fn signed_build_emits_both_mfbnote_and_mfbsign_disjoint() {
        let mut img = image(vec![0xc3]);
        img.signing_metadata = Some(b"{\"format\":\"mfb-signing-v1\"}".to_vec());
        let bytes = write_executable(&img, false, None, None).expect("link");
        let (note_va, note_sz) = section_extent(&bytes, b".mfbnote").expect(".mfbnote present");
        let (sign_va, sign_sz) = section_extent(&bytes, b".mfbsign").expect(".mfbsign present");
        // Disjoint virtual ranges (neither starts inside the other's extent).
        assert!(
            sign_va >= note_va + note_sz || note_va >= sign_va + sign_sz,
            ".mfbnote [{note_va:#x}, +{note_sz:#x}) and .mfbsign [{sign_va:#x}, +{sign_sz:#x}) overlap"
        );
    }

    #[test]
    fn minimal_text_only_image_links() {
        let img = image(vec![0xc3]); // ret
        let bytes = write_executable(&img, false, None, None).expect("link");
        assert_eq!(&bytes[0..2], b"MZ");
        // Three sections: .text, the unconditional .reloc the image needs to be
        // ASLR-capable (bug-504 — DYNAMIC_BASE is ignored without a base
        // relocation directory), and the unconditional .mfbnote provenance
        // section (bug-433); no import directories.
        let e_lfanew = le_u32(&bytes, 0x3C) as usize;
        assert_eq!(le_u16(&bytes, e_lfanew + 6), 3);
        assert!(
            section_named(&bytes, b".reloc").is_some(),
            "even a text-only image must carry .reloc, or it loads at a fixed base"
        );
    }

    #[test]
    fn entry_not_in_text_is_rejected() {
        let mut img = image(vec![0xc3]);
        img.symbols[0].section = EncodedSection::Data;
        assert!(write_executable(&img, false, None, None)
            .expect_err("entry not in text")
            .contains("does not resolve to text"));
    }

    #[test]
    fn internal_call_patches_rel32() {
        // _start at 0 does `call helper`; helper at offset 8. The E8 disp32 field
        // is at text offset 1, so site_rva = text_rva+1, next = +5, and
        // rel32 = helper_rva - (text_rva+1+4) = 8 - 5 = 3.
        let mut text = vec![0xe8, 0, 0, 0, 0]; // call rel32 (disp at 1)
        text.extend_from_slice(&[0xcc, 0xcc, 0xcc]); // pad to offset 8
        text.push(0xc3); // helper: ret
        let mut img = image(text);
        img.symbols.push(EncodedSymbol {
            name: "helper".to_string(),
            section: EncodedSection::Text,
            offset: 8,
        });
        img.relocations.push(EncodedRelocation {
            offset: 1,
            target: "helper".to_string(),
            kind: "call_pc32".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        let bytes = write_executable(&img, false, None, None).expect("link");
        let text_rva = le_u32(&bytes, le_u32(&bytes, 0x3C) as usize + 4 + 20 + 20 + 12); // .text vaddr
        let patched = read_at_rva(&bytes, text_rva + 1, 4);
        assert_eq!(
            i32::from_le_bytes([patched[0], patched[1], patched[2], patched[3]]),
            3
        );
    }

    /// The plan's behavioral outcome (§1): an `ExitProcess(42)` image through a
    /// one-entry `kernel32.dll` IAT. Entry is `mov ecx, 42; call [thunk]`.
    fn exit_process_42_image() -> EncodedImage {
        // 0: B9 2A 00 00 00   mov ecx, 42
        // 5: E8 xx xx xx xx   call rel32 -> ExitProcess thunk (disp at 6)
        let mut text = vec![0xb9, 0x2a, 0x00, 0x00, 0x00, 0xe8, 0, 0, 0, 0];
        text.extend_from_slice(&[0xcc]); // pad
        EncodedImage {
            text,
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![EncodedSymbol {
                name: "_start".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: vec![EncodedRelocation {
                offset: 6,
                target: "ExitProcess".to_string(),
                kind: "call_pc32".to_string(),
                binding: "external".to_string(),
                library: Some("kernel32.dll".to_string()),
            }],
            imports: vec![EncodedImport {
                library: "kernel32.dll".to_string(),
                symbol: "ExitProcess".to_string(),
                kind: ImportKind::Function,
                version: None,
            }],
            entry: "_start".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
        }
    }

    #[test]
    fn exit_process_image_has_text_and_idata_and_bound_call() {
        let bytes = write_executable(&exit_process_42_image(), false, None, None).expect("link");
        let e_lfanew = le_u32(&bytes, 0x3C) as usize;
        // Four sections: .text (with the appended thunk), .idata, the
        // unconditional .reloc that makes the image ASLR-capable (bug-504), and
        // the unconditional .mfbnote provenance section (bug-433).
        assert_eq!(le_u16(&bytes, e_lfanew + 6), 4);
        // Data directory [1] Import and [12] IAT are populated.
        let dd = e_lfanew + 4 + 20 + 112;
        let import_rva = le_u32(&bytes, dd + 8);
        let iat_rva = le_u32(&bytes, dd + 12 * 8);
        assert_ne!(import_rva, 0, "Import directory present");
        assert_ne!(iat_rva, 0, "IAT present");

        // The IAT slot points at the hint/name entry, which names "ExitProcess".
        let slot = le_u64(&bytes, {
            // resolve iat_rva to a file offset via the section table
            let n = le_u16(&bytes, e_lfanew + 6) as usize;
            let st = e_lfanew + 4 + 20 + 240;
            let mut fo = 0usize;
            for i in 0..n {
                let s = st + i * 40;
                let va = le_u32(&bytes, s + 12);
                let vs = le_u32(&bytes, s + 8);
                if iat_rva >= va && iat_rva < va + vs {
                    fo = (le_u32(&bytes, s + 20) + (iat_rva - va)) as usize;
                }
            }
            fo
        });
        let hint_name = read_at_rva(&bytes, slot as u32 + 2, 11);
        assert_eq!(&hint_name, b"ExitProcess");

        // The .text thunk is FF 25 (jmp [rip+disp]) jumping to the IAT slot.
        let text_va = le_u32(&bytes, e_lfanew + 4 + 20 + 20 + 12);
        // Thunk is appended right after the original 11-byte text.
        let thunk_rva = text_va + 11;
        let thunk = read_at_rva(&bytes, thunk_rva, 6);
        assert_eq!(&thunk[0..2], &[0xff, 0x25], "FF 25 jmp [rip+disp32]");
        let disp = i32::from_le_bytes([thunk[2], thunk[3], thunk[4], thunk[5]]);
        assert_eq!(
            (thunk_rva as i64 + 6 + disp as i64) as u32,
            iat_rva,
            "thunk jumps through the IAT slot"
        );

        // The external call's rel32 targets the thunk.
        let call = read_at_rva(&bytes, text_va + 6, 4);
        let call_rel = i32::from_le_bytes([call[0], call[1], call[2], call[3]]);
        assert_eq!(
            (text_va as i64 + 6 + 4 + call_rel as i64) as u32,
            thunk_rva,
            "external call targets the FF 25 thunk"
        );
    }

    #[test]
    fn imports_grouped_by_dll_first_seen_order() {
        let mut img = exit_process_42_image();
        img.imports.push(EncodedImport {
            library: "kernel32.dll".to_string(),
            symbol: "WriteFile".to_string(),
            kind: ImportKind::Function,
            version: None,
        });
        img.imports.insert(
            0,
            EncodedImport {
                library: "bcrypt.dll".to_string(),
                symbol: "BCryptGenRandom".to_string(),
                kind: ImportKind::Function,
                version: None,
            },
        );
        let groups = group_imports_by_dll(&img);
        assert_eq!(groups[0].0, "bcrypt.dll");
        assert_eq!(groups[1].0, "kernel32.dll");
        assert_eq!(groups[1].1, vec!["ExitProcess", "WriteFile"]);
    }

    /// A read-only prefix (`.rdata`) plus writable data (`.data`) both emit their
    /// own section, laid out contiguously so a data symbol's RVA is the same in
    /// either partition (§4.4).
    #[test]
    fn rdata_and_data_sections_are_both_emitted() {
        let mut img = image(vec![0xc3]);
        // rodata_size must be SECTION_ALIGNMENT-aligned so the .rdata/.data split
        // keeps data symbol RVAs contiguous (the debug_assert at §4.4).
        img.data = vec![0xAB; SECTION_ALIGNMENT as usize + 16];
        img.rodata_size = SECTION_ALIGNMENT as usize;
        let bytes = write_executable(&img, false, None, None).expect("link");
        let e_lfanew = le_u32(&bytes, 0x3C) as usize;
        // .text + .rdata + .data + the unconditional .reloc (bug-504) + the
        // unconditional .mfbnote (bug-433) == 5 sections.
        assert_eq!(le_u16(&bytes, e_lfanew + 6), 5);
        // The .data section's first byte round-trips.
        let sect_table = e_lfanew + 4 + 20 + 240;
        let data_vaddr = le_u32(&bytes, sect_table + 2 * 40 + 12);
        assert_eq!(read_at_rva(&bytes, data_vaddr, 1), vec![0xAB]);
    }

    /// A `Data`-kind import carries no `FF 25` thunk — `append_thunks` skips it —
    /// yet still occupies an IAT slot through `build_idata`.
    #[test]
    fn data_import_is_skipped_by_thunks() {
        let mut img = exit_process_42_image();
        img.imports.push(EncodedImport {
            library: "kernel32.dll".to_string(),
            symbol: "SomeDataGlobal".to_string(),
            kind: ImportKind::Data,
            version: None,
        });
        let bytes = write_executable(&img, false, None, None).expect("link");
        assert_eq!(&bytes[0..2], b"MZ");
    }

    #[test]
    fn external_call_with_no_iat_slot_is_rejected() {
        let mut img = image(vec![0xe8, 0, 0, 0, 0, 0xcc, 0xcc, 0xcc]);
        img.relocations.push(EncodedRelocation {
            offset: 1,
            target: "Missing".to_string(),
            kind: "call_pc32".to_string(),
            binding: "external".to_string(),
            library: Some("kernel32.dll".to_string()),
        });
        let err = write_executable(&img, false, None, None).expect_err("no thunk");
        assert!(err.contains("cannot bind external call"), "{err}");
    }

    fn image_with_import_and_data_reloc(target: &str) -> EncodedImage {
        // mov eax, [rip+disp32] ; ret   (disp32 field at text offset 2)
        let mut img = image(vec![0x8b, 0x05, 0, 0, 0, 0, 0xc3]);
        img.imports.push(EncodedImport {
            library: "kernel32.dll".to_string(),
            symbol: "ExitProcess".to_string(),
            kind: ImportKind::Function,
            version: None,
        });
        img.relocations.push(EncodedRelocation {
            offset: 2,
            target: target.to_string(),
            kind: "data_pc32".to_string(),
            binding: "external".to_string(),
            library: Some("kernel32.dll".to_string()),
        });
        img
    }

    #[test]
    fn external_data_reloc_binds_to_the_iat_slot() {
        // The function import populates its IAT slot via `append_thunks`; an
        // external data reloc to it binds to that slot RVA.
        let bytes = write_executable(
            &image_with_import_and_data_reloc("ExitProcess"),
            false,
            None,
            None,
        )
        .expect("link");
        assert_eq!(&bytes[0..2], b"MZ");
    }

    #[test]
    fn external_data_reloc_with_no_slot_is_rejected() {
        let err = write_executable(
            &image_with_import_and_data_reloc("MissingData"),
            false,
            None,
            None,
        )
        .expect_err("unbound data");
        assert!(err.contains("cannot bind external data"), "{err}");
    }

    #[test]
    fn unsupported_relocation_kind_is_rejected() {
        let mut img = image(vec![0xc3]);
        img.relocations.push(EncodedRelocation {
            offset: 0,
            target: "_start".to_string(),
            kind: "abs64".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        let err = write_executable(&img, false, None, None).expect_err("unsupported");
        assert!(err.contains("does not support relocation"), "{err}");
    }

    #[test]
    fn relocation_offset_past_text_end_is_rejected() {
        let mut img = image(vec![0xc3, 0xc3]);
        img.relocations.push(EncodedRelocation {
            offset: 1, // 1 + 4 = 5 > text.len() 2
            target: "_start".to_string(),
            kind: "call_pc32".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        let err = write_executable(&img, false, None, None).expect_err("out of range");
        assert!(err.contains("out of range"), "{err}");
    }

    #[test]
    fn runnable_exit42_image_links_to_a_pe() {
        let bytes = write_executable(&runnable_exit42_image(), false, None, None)
            .expect("link runnable exit42");
        assert_eq!(&bytes[0..2], b"MZ");
    }

    /// bug-433: every Windows `.exe` — like every ELF/Mach-O the linker emits —
    /// carries the unconditional `MFBasic\0` provenance marker, whose 16-byte
    /// descriptor is the shared `note.rs` payload, in a dedicated `.mfbnote`
    /// section. Even a bare console image must carry it.
    #[test]
    fn provenance_marker_emitted_unconditionally() {
        use crate::os::note::{mfb_note_descriptor, MFB_NOTE_OWNER};
        let bytes = write_executable(&image(vec![0xc3]), false, None, None).expect("link");

        // The marker is a discoverable section-table entry named ".mfbnote", not a
        // stray byte match — locate it in the section table and read its body at
        // that section's RVA.
        let e_lfanew = le_u32(&bytes, 0x3C) as usize;
        let n = le_u16(&bytes, e_lfanew + 6) as usize;
        let sect_table = e_lfanew + 4 + 20 + 240;
        let mfbnote_rva = (0..n)
            .map(|i| sect_table + i * 40)
            .find(|&s| &bytes[s..s + 8] == section_name(".mfbnote").as_slice())
            .map(|s| le_u32(&bytes, s + 12))
            .expect(".mfbnote section present in the table");

        // Body: the MFBasic\0 owner followed verbatim by the shared 16-byte
        // descriptor — the identical owner+descriptor framing ELF/Mach-O use.
        let descriptor = mfb_note_descriptor();
        let body = read_at_rva(&bytes, mfbnote_rva, MFB_NOTE_OWNER.len() + descriptor.len());
        assert_eq!(&body[..MFB_NOTE_OWNER.len()], MFB_NOTE_OWNER);
        assert_eq!(&body[MFB_NOTE_OWNER.len()..], descriptor.as_slice());
    }

    /// plan-66-I: `gui = true` links the GUI subsystem (WINDOWS_GUI = 2), `false`
    /// the console subsystem (WINDOWS_CUI = 3). Subsystem is at optional-header
    /// offset 68.
    #[test]
    fn gui_flag_selects_the_pe_subsystem() {
        let gui = write_executable(&image(vec![0xc3]), true, None, None).expect("gui");
        let opt = le_u32(&gui, 0x3C) as usize + 4 + 20;
        assert_eq!(le_u16(&gui, opt + 68), 2, "WINDOWS_GUI");

        let cui = write_executable(&image(vec![0xc3]), false, None, None).expect("cui");
        let opt = le_u32(&cui, 0x3C) as usize + 4 + 20;
        assert_eq!(le_u16(&cui, opt + 68), 3, "WINDOWS_CUI");
    }
}
