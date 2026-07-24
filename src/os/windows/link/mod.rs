//! PE32+ executable linker (plan-47-C Phase 3).
//!
//! Binds an [`EncodedImage`] into a finished PE32+ `.exe`: builds `.idata`
//! (import directory + ILTs + IATs + hint/name table) from `image.imports`,
//! appends one `FF 25` IAT thunk per imported function to `.text`, patches every
//! relocation, and hands the laid-out sections to [`pe::write_image`].
//!
//! Mirrors `src/os/linux/link/mod.rs`: the x86 `rel32 = target − (site+4)` math
//! and the `FF 25 disp32` thunk are byte-for-byte the ELF path's, with the IAT
//! slot standing in for the GOT slot. Determinism: imports are grouped by DLL on
//! first appearance, never via `HashMap` iteration (§1 / bug-87).
//!
//! The parent `windows` module carries the `dead_code` allow (47-D removes it)
//! since nothing calls `write_executable` until the backend is wired.

mod pe;

use crate::arch::aarch64::encode::{EncodedImage, EncodedSection, ImportKind};
use pe::{
    align_up, section_name, size_of_headers, ImportDirectories, Section, IMAGE_BASE, SCN_DATA,
    SCN_IDATA, SCN_RDATA, SCN_TEXT,
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
        let slot = *slot_rva.get(&import.symbol).ok_or_else(|| {
            format!(
                "windows linker: import '{}' has no IAT slot",
                import.symbol
            )
        })?;
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
pub(crate) fn write_executable(image: &EncodedImage) -> Result<Vec<u8>, String> {
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

    // Count sections to size the headers, then lay out RVAs/file offsets.
    let section_count = 1 + has_rdata as usize + has_data as usize + has_idata as usize;
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

    let dirs = idata
        .as_ref()
        .map(|i| ImportDirectories {
            import: i.import_dir,
            iat: i.iat,
        })
        .unwrap_or_default();

    Ok(pe::write_image(
        &sections,
        text_rva + entry_offset as u32,
        dirs,
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
fn write_rel32(text: &mut [u8], offset: usize, target_rva: u32, site_rva: u32) -> Result<(), String> {
    if offset + 4 > text.len() {
        return Err(format!("windows linker: relocation offset {offset} out of range"));
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
    use crate::arch::aarch64::encode::{
        EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind,
    };

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

    #[test]
    fn minimal_text_only_image_links() {
        let img = image(vec![0xc3]); // ret
        let bytes = write_executable(&img).expect("link");
        assert_eq!(&bytes[0..2], b"MZ");
        // One section (.text), no import directories.
        let e_lfanew = le_u32(&bytes, 0x3C) as usize;
        assert_eq!(le_u16(&bytes, e_lfanew + 6), 1);
    }

    #[test]
    fn entry_not_in_text_is_rejected() {
        let mut img = image(vec![0xc3]);
        img.symbols[0].section = EncodedSection::Data;
        assert!(write_executable(&img)
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
        let bytes = write_executable(&img).expect("link");
        let text_rva = le_u32(&bytes, le_u32(&bytes, 0x3C) as usize + 4 + 20 + 20 + 12); // .text vaddr
        let patched = read_at_rva(&bytes, text_rva + 1, 4);
        assert_eq!(i32::from_le_bytes([patched[0], patched[1], patched[2], patched[3]]), 3);
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
        let bytes = write_executable(&exit_process_42_image()).expect("link");
        let e_lfanew = le_u32(&bytes, 0x3C) as usize;
        // Two sections: .text (with the appended thunk) and .idata.
        assert_eq!(le_u16(&bytes, e_lfanew + 6), 2);
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
}
