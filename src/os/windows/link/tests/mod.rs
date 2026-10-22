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
        debug_symbols: false,
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
        debug_symbols: false,
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
    let bytes = write_executable(&runnable_exit42_image(), false, None, None).expect("link exit42");
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
        let bytes =
            write_executable(&runnable_exit42_image(), gui, None, None).expect("link exit42");
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
        assert_ne!(
            reloc_size, 0,
            "BASERELOC directory must be populated (gui={gui})"
        );
        let reloc = section_named(&bytes, b".reloc").expect(".reloc section present");
        // The directory points at the section start and covers exactly its body.
        let n = le_u16(&bytes, e_lfanew + 6) as usize;
        let sect_table = opt + 240;
        let reloc_hdr = (0..n)
            .map(|i| sect_table + i * 40)
            .find(|&s| &bytes[s..s + 6] == b".reloc")
            .expect(".reloc header");
        assert_eq!(
            le_u32(&bytes, reloc_hdr + 12),
            reloc_rva,
            "[5].RVA = .reloc RVA"
        );
        assert_eq!(
            le_u32(&bytes, reloc_hdr + 8),
            reloc_size,
            "[5].Size = .reloc VirtualSize"
        );
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
            assert!(
                block_size >= 8 && block_size % 4 == 0,
                "SizeOfBlock {block_size}"
            );
            assert_eq!(page % 0x1000, 0, "block page {page:#x} is page-aligned");
            assert!(
                page < size_of_image,
                "block page {page:#x} inside the image"
            );
            for entry in (8..block_size).step_by(2) {
                let kind = le_u16(&reloc, off + entry) >> 12;
                assert!(
                    kind == 0 || kind == 10,
                    "entry type {kind} (ABSOLUTE or DIR64)"
                );
            }
            off += block_size;
            blocks += 1;
        }
        assert_eq!(
            off, reloc_size as usize,
            "blocks tile the directory exactly"
        );
        assert!(blocks >= 1, "at least one relocation block");
    }
}

/// bug-432: a signed Windows build emits the `mfb-signing-v1` blob verbatim in
/// an `.mfbsign` PE section (the unified 8-char name shared with ELF/Mach-O).
#[test]
fn signing_metadata_emits_mfbsign_section() {
    let mut img = image(vec![0xc3]); // ret
    img.signing_metadata = Some(crate::arch::image::ExecutableSigning::unsealed(
        b"{\"format\":\"mfb-signing-v1\"}",
    ));
    let bytes = write_executable(&img, false, None, None).expect("link");
    let section = section_named(&bytes, b".mfbsign").expect(".mfbsign section present");
    let metadata = &img.signing_metadata.as_ref().unwrap().metadata;
    // The section body carries the blob verbatim (no PE-specific header). The
    // raw body may be FILE_ALIGNMENT-padded, so scan for the blob within it.
    assert!(
        section
            .windows(metadata.len())
            .any(|w| w == metadata.as_slice()),
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
    img.signing_metadata = Some(crate::arch::image::ExecutableSigning::unsealed(
        b"{\"format\":\"mfb-signing-v1\"}",
    ));
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
        debug_symbols: false,
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

// ===== plan-166: `--debug` COFF symbol table =====

/// `(name, value, section_number, type, storage_class, aux)` for every COFF
/// symbol record, read the way `llvm-nm` does: from the COFF header's
/// `PointerToSymbolTable` / `NumberOfSymbols`, with long names through the
/// string table that follows the records.
fn coff_symbols(image: &[u8]) -> Vec<(String, u32, i16, u16, u8, u8)> {
    let coff = le_u32(image, 0x3C) as usize + 4;
    let pointer = le_u32(image, coff + 8) as usize;
    let count = le_u32(image, coff + 12) as usize;
    let strings = pointer + count * 18;
    assert_eq!(
        le_u32(image, strings) as usize,
        image.len() - strings,
        "the string table's size field covers it to the file end"
    );
    (0..count)
        .map(|index| {
            let at = pointer + index * 18;
            let name = if le_u32(image, at) == 0 {
                let start = strings + le_u32(image, at + 4) as usize;
                let end = start + image[start..].iter().position(|b| *b == 0).unwrap();
                String::from_utf8(image[start..end].to_vec()).unwrap()
            } else {
                let raw = &image[at..at + 8];
                let end = raw.iter().position(|b| *b == 0).unwrap_or(8);
                String::from_utf8(raw[..end].to_vec()).unwrap()
            };
            (
                name,
                le_u32(image, at + 8),
                le_u16(image, at + 12) as i16,
                le_u16(image, at + 14),
                image[at + 16],
                image[at + 17],
            )
        })
        .collect()
}

/// The `(name, virtual address)` of the 1-based section `number`.
fn section_by_number(image: &[u8], number: i16) -> (String, u32) {
    let e_lfanew = le_u32(image, 0x3C) as usize;
    let s = e_lfanew + 4 + 20 + 240 + (number as usize - 1) * 40;
    let raw = &image[s..s + 8];
    let end = raw.iter().position(|b| *b == 0).unwrap_or(8);
    (
        String::from_utf8(raw[..end].to_vec()).unwrap(),
        le_u32(image, s + 12),
    )
}

/// A function with a long name, a constant with an inline (≤ 8 byte) name in
/// `.rdata`, and a writable global in `.data`, `--debug` on.
fn debug_image() -> EncodedImage {
    let mut img = image(vec![0xc3, 0x90, 0x90, 0x90, 0xc3]);
    img.symbols.push(EncodedSymbol {
        name: "_mfb_fn_helper".to_string(),
        section: EncodedSection::Text,
        offset: 4,
    });
    img.data = vec![0; SECTION_ALIGNMENT as usize + 16];
    img.rodata_size = SECTION_ALIGNMENT as usize;
    img.symbols.push(EncodedSymbol {
        name: "_c".to_string(),
        section: EncodedSection::Data,
        offset: 8,
    });
    img.symbols.push(EncodedSymbol {
        name: "_mfb_rt_main_arena".to_string(),
        section: EncodedSection::Data,
        offset: SECTION_ALIGNMENT as usize + 8,
    });
    img.debug_symbols = true;
    img
}

#[test]
fn debug_symbols_fill_coff_header_and_table() {
    let img = debug_image();
    let bytes = write_executable(&img, false, None, None).expect("link");
    let coff = le_u32(&bytes, 0x3C) as usize + 4;
    let pointer = le_u32(&bytes, coff + 8);
    assert_eq!(
        pointer % FILE_ALIGNMENT,
        0,
        "the table starts FileAlignment-aligned"
    );
    assert_eq!(le_u32(&bytes, coff + 12), 4, "NumberOfSymbols");
    // The table is past every section's raw data.
    let e_lfanew = le_u32(&bytes, 0x3C) as usize;
    for index in 0..le_u16(&bytes, e_lfanew + 6) as usize {
        let s = e_lfanew + 4 + 20 + 240 + index * 40;
        assert!(le_u32(&bytes, s + 20) + le_u32(&bytes, s + 16) <= pointer);
    }

    let symbols = coff_symbols(&bytes);
    let names: Vec<_> = symbols.iter().map(|symbol| symbol.0.as_str()).collect();
    assert_eq!(
        names,
        ["_start", "_mfb_fn_helper", "_c", "_mfb_rt_main_arena"]
    );
    let (text_rva, data_base_rva) = (section_by_number(&bytes, 1).1, {
        let (name, rva) = section_by_number(&bytes, 2);
        assert_eq!(name, ".rdata");
        rva
    });
    for (name, value, number, type_, class, aux) in &symbols {
        let (section, section_rva) = section_by_number(&bytes, *number);
        let expected_section = match name.as_str() {
            "_start" | "_mfb_fn_helper" => ".text",
            "_c" => ".rdata",
            _ => ".data",
        };
        assert_eq!(section, expected_section, "{name}: SectionNumber");
        // `llvm-nm` prints ImageBase + section RVA + Value: that must be the
        // address relocations resolve the same name to.
        assert_eq!(
            section_rva + value,
            symbol_rva(&img, name, text_rva, data_base_rva).unwrap(),
            "{name}: Value"
        );
        let expected_type = if expected_section == ".text" { 0x20 } else { 0 };
        assert_eq!(*type_, expected_type, "{name}: Type");
        assert_eq!((*class, *aux), (2, 0), "{name}: EXTERNAL, no aux records");
    }
    // `_start` and `_c` fit the inline 8-byte field, NUL-padded; the longer
    // names go through the string table, the first of them at offset 4 (past
    // the size field).
    let record = |index: usize| pointer as usize + index * 18;
    assert_eq!(&bytes[record(0)..record(0) + 8], b"_start\0\0");
    assert_eq!(le_u32(&bytes, record(1)), 0);
    assert_eq!(le_u32(&bytes, record(1) + 4), 4);
    assert_eq!(&bytes[record(2)..record(2) + 8], b"_c\0\0\0\0\0\0");
}

#[test]
fn debug_symbols_off_leaves_header_zero() {
    let img = EncodedImage {
        debug_symbols: false,
        ..debug_image()
    };
    let bytes = write_executable(&img, false, None, None).expect("link");
    let coff = le_u32(&bytes, 0x3C) as usize + 4;
    assert_eq!(le_u32(&bytes, coff + 8), 0, "PointerToSymbolTable");
    assert_eq!(le_u32(&bytes, coff + 12), 0, "NumberOfSymbols");
    assert_eq!(
        bytes.len() % FILE_ALIGNMENT as usize,
        0,
        "nothing past the last section"
    );
    assert!(!bytes.windows(14).any(|w| w == b"_mfb_fn_helper"));

    // The table changes nothing else: the same image with `--debug` symbols
    // is these bytes, plus the appended table, with only the two COFF symbol
    // fields differing in the headers.
    let with_table = write_executable(&debug_image(), false, None, None).expect("link");
    assert!(with_table.len() > bytes.len());
    let fields = coff + 8..coff + 16;
    assert_eq!(with_table[..fields.start], bytes[..fields.start]);
    assert_eq!(with_table[fields.end..bytes.len()], bytes[fields.end..]);
}

#[test]
fn signed_debug_build_keeps_mfbsign_and_seals() {
    let mut img = debug_image();
    img.signing_metadata = Some(crate::arch::image::ExecutableSigning {
        metadata: format!(
            "{{\"contentSignature\":\"{}\"}}",
            crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER
        )
        .into_bytes(),
        signing_private: Some(mfb_repository::crypto::generate_keypair().1),
    });
    // The seal runs after the table is appended, and succeeds.
    let bytes = write_executable(&img, false, None, None).expect("link and seal");
    let binary = crate::os::inspect::inspect(&bytes).expect("inspect reads it back");
    let blob = binary.signing.expect(".mfbsign found by name");
    let section = section_named(&bytes, b".mfbsign").unwrap();
    assert_eq!(&bytes[blob.clone()], &section[..blob.len()]);
    assert_eq!(binary.covered_end, bytes.len(), "the seal covers the table");
    assert_eq!(coff_symbols(&bytes).len(), 4);
    assert!(!bytes
        .windows(crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER.len())
        .any(|w| w == crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER.as_bytes()));
}

/// A debug symbol table entry for a data symbol past the read-only prefix of
/// an image with no writable data has no section to name, and is refused
/// rather than given a wrong section number; a defined-symbol lookup for a
/// name the image does not define is refused by name.
#[test]
fn a_symbol_with_no_home_is_refused() {
    let mut img = image(vec![0xc3]);
    img.data = vec![0; 8];
    img.rodata_size = 8;
    img.symbols.push(EncodedSymbol {
        name: "_end".to_string(),
        section: EncodedSection::Data,
        offset: 8,
    });
    let err = coff_symbol_table(&img, 8, true, false)
        .err()
        .expect("no section holds offset 8");
    assert_eq!(
        err,
        "debug symbols: data symbol '_end' at offset 8 lands in no section"
    );
    assert_eq!(
        symbol_rva(&img, "_nowhere", 0x1000, 0x2000),
        Err("windows linker: undefined symbol '_nowhere'".to_string())
    );
    assert_eq!(symbol_rva(&img, "_end", 0x1000, 0x2000), Ok(0x2008));
}
