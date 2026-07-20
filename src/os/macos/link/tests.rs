use super::commands::bind_info;
use super::*;
use crate::arch::aarch64::encode::{EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind};

fn import(library: &str, symbol: &str) -> EncodedImport {
    EncodedImport {
        library: library.to_string(),
        symbol: symbol.to_string(),
        kind: ImportKind::Function,
        version: None,
    }
}

#[test]
fn patches_external_data_relocations_to_got_entry() {
    let mut text = vec![
        0x00, 0x00, 0x00, 0x90, // adrp x0, symbol
        0x00, 0x00, 0x00, 0x91, // add x0, x0, pageoff(symbol)
    ];
    let image = EncodedImage {
        text: text.clone(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![
            EncodedRelocation {
                offset: 0,
                target: "_mach_task_self_".to_string(),
                kind: "page21".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
            EncodedRelocation {
                offset: 4,
                target: "_mach_task_self_".to_string(),
                kind: "pageoff12".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
        ],
        imports: vec![import("libSystem", "_mach_task_self_")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let text_vmaddr = VM_BASE + 0x4000;
    let locations =
        append_import_stubs(&mut text, &image, text_vmaddr, 0x4000, 0).expect("import stubs");

    let data_vmaddr = text_vmaddr + text.len() as u64;
    patch_relocations(
        &mut text,
        &image,
        text_vmaddr,
        data_vmaddr,
        data_vmaddr,
        0,
        &locations,
    )
    .expect("relocations");

    assert!(locations.got_entries.contains_key("_mach_task_self_"));
}

#[test]
fn bind_info_uses_uleb_ordinal_past_fifteen() {
    // A 16th distinct library forces the ULEB ordinal path (BIND_OPCODE
    // SET_DYLIB_ORDINAL_ULEB, 0x80) instead of the packed immediate form.
    // `bind_info` takes the library list directly, so hand-build 16 libraries
    // and place the single import in the last one (ordinal 16).
    let libraries: Vec<(String, String)> = (1..=16)
        .map(|i| (format!("lib{i}"), format!("/usr/lib/lib{i}.dylib")))
        .collect();
    let image = EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: vec![import("lib16", "_late")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    assert_eq!(library_ordinal(&libraries, "lib16").unwrap(), 16);
    let bind = bind_info(&image, &libraries);
    // 0x80 = SET_DYLIB_ORDINAL_ULEB, then a single ULEB byte 16 (0x10).
    assert_eq!(bind[0], 0x80);
    assert_eq!(bind[1], 0x10);
    // Followed by BIND_OPCODE_SET_SYMBOL (0x40) and the null-terminated name.
    assert_eq!(bind[2], 0x40);
    assert!(bind
        .windows(b"_late\0".len())
        .any(|window| window == b"_late\0"));
}

#[test]
fn import_libraries_assigns_one_ordinal_per_distinct_library() {
    let image = EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: vec![
            import("libSystem", "_exit"),
            import("Network", "_nw_path_monitor_create"),
            import("libSystem", "_write"),
        ],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let libraries = import_libraries(&image).expect("libraries");
    assert_eq!(libraries.len(), 2);
    assert_eq!(library_ordinal(&libraries, "libSystem").unwrap(), 1);
    assert_eq!(library_ordinal(&libraries, "Network").unwrap(), 2);
    // The bind stream tags _nw_path_monitor_create with dylib ordinal 2.
    let bind = bind_info(&image, &libraries);
    assert!(bind.contains(&0x12)); // SET_DYLIB_ORDINAL_IMM(2)
}

#[test]
fn rejects_initializer_without_text_symbol() {
    // plan-linker.md §5.3: an initializer that names no internal text symbol
    // must error rather than be silently dropped (mirrors the Linux backend).
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: vec!["_missing".to_string()],
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let error = write_executable(dir.path(), "init", &image)
        .expect_err("dangling initializer must be rejected");
    assert!(error.contains("does not resolve to a text symbol"));
}

// S_MOD_INIT_FUNC_POINTERS end to end on the no-imports path (plan-linker.md
// §7.5): a self-contained program whose initializer exits 0 via a direct
// syscall while `_main` would exit 7. Exit 0 proves dyld ran the rebased
// `__mod_init_func` pointer before `LC_MAIN`; a missing or mis-rebased pointer
// would let `_main` run (exit 7) or crash.
#[cfg(target_os = "macos")]
#[test]
fn runs_initializer_before_entry_without_imports() {
    let words: [u32; 6] = [
        0xD280_0000, // _init0: movz x0, #0
        0xD280_0030, //         movz x16, #1   (SYS_exit)
        0xD400_1001, //         svc  #0x80     -> exit(0)
        0xD280_00E0, // _main:  movz x0, #7
        0xD280_0030, //         movz x16, #1
        0xD400_1001, //         svc  #0x80     -> exit(7)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_init0".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 12,
            },
        ],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: vec!["_init0".to_string()],
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!("mfb_modinit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = write_executable(&dir, "modinit", &image).expect("link initializer executable");
    let status = std::process::Command::new(&path)
        .status()
        .expect("run initializer executable");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "the __mod_init_func initializer must run (exit 0) before _main (exit 7)"
    );
}

// The combined path: a GOT (imported `_exit`) and a `__mod_init_func` share the
// __DATA_CONST segment. The initializer calls the imported `_exit(0)`; `_main`
// would call `_exit(7)`. Exit 0 proves bind (GOT) and rebase (mod-init) coexist
// correctly in one segment.
#[cfg(target_os = "macos")]
#[test]
fn runs_initializer_before_entry_with_imports() {
    let words: [u32; 4] = [
        0xD280_0000, // _init0: movz x0, #0
        0x9400_0000, //         bl _exit        (external branch26, patched)
        0xD280_00E0, // _main:  movz x0, #7
        0x9400_0000, //         bl _exit        (external branch26, patched)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_init0".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 8,
            },
        ],
        relocations: vec![
            EncodedRelocation {
                offset: 4,
                target: "_exit".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
            EncodedRelocation {
                offset: 12,
                target: "_exit".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
        ],
        imports: vec![import("libSystem", "_exit")],
        entry: "_main".to_string(),
        initializers: vec!["_init0".to_string()],
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!("mfb_modinit_imp_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path =
        write_executable(&dir, "modinitimp", &image).expect("link initializer+import executable");
    let status = std::process::Command::new(&path)
        .status()
        .expect("run initializer+import executable");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "the initializer's imported _exit(0) must run before _main's _exit(7)"
    );
}

#[test]
fn writes_mfb_sign_section_to_mach_o() {
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: Some(br#"{"owner":"alice"}"#.to_vec()),
        rpaths: Vec::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "signed", &image).expect("link signed mach-o");
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes
        .windows(b"__MFB".len())
        .any(|window| window == b"__MFB"));
    assert!(bytes
        .windows(b"__sign".len())
        .any(|window| window == b"__sign"));
    assert!(bytes
        .windows(br#"{"owner":"alice"}"#.len())
        .any(|window| window == br#"{"owner":"alice"}"#));
}

/// The `MFBasic\0` `LC_NOTE`'s out-of-line payload (plan-43), located the way
/// `otool -l` does: walk the load commands, match `cmd == LC_NOTE` and the
/// `data_owner`, then read `offset`/`size` out of the file. Also returns the
/// payload's file offset so callers can prove it lies in the signed prefix.
fn mfb_note(bytes: &[u8]) -> Option<(usize, Vec<u8>)> {
    let read_u32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let read_u64 = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let count = read_u32(16) as usize;
    let mut command = 32;
    for _ in 0..count {
        let size = read_u32(command + 4) as usize;
        if read_u32(command) == 0x31 {
            assert_eq!(size, NOTE_COMMAND_SIZE);
            let mut data_owner = [0u8; 16];
            data_owner[..MFB_NOTE_OWNER.len()].copy_from_slice(MFB_NOTE_OWNER);
            if bytes[command + 8..command + 24] == data_owner {
                let offset = read_u64(command + 24) as usize;
                let length = read_u64(command + 32) as usize;
                return Some((offset, bytes[offset..offset + length].to_vec()));
            }
        }
        command += size;
    }
    None
}

/// The file offset of `LC_CODE_SIGNATURE`'s blob — the ad-hoc signature's
/// `codeLimit`. Every byte below it is covered by a page hash.
fn code_signature_offset(bytes: &[u8]) -> usize {
    let read_u32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let count = read_u32(16) as usize;
    let mut command = 32;
    for _ in 0..count {
        if read_u32(command) == 0x1d {
            return read_u32(command + 8) as usize;
        }
        command += read_u32(command + 4) as usize;
    }
    panic!("LC_CODE_SIGNATURE must be present");
}

/// plan-43: every emitted Mach-O carries an unconditional `LC_NOTE` whose
/// `data_owner` is `MFBasic\0` and whose out-of-line payload is the shared
/// descriptor — placed below `codeLimit`, so the ad-hoc signature covers it and
/// stays valid.
#[test]
fn mach_o_carries_the_mfbasic_provenance_note_inside_the_signed_region() {
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![text_symbol("_main", 0)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "noted", &image).expect("link mach-o");
    let bytes = std::fs::read(path).unwrap();
    let (offset, descriptor) = mfb_note(&bytes).expect("LC_NOTE owned by MFBasic");
    assert_eq!(descriptor, mfb_note_descriptor());
    assert_eq!(offset % 16, 0, "the payload stays 16-byte aligned");
    assert!(
        offset + descriptor.len() <= code_signature_offset(&bytes),
        "the payload must lie inside the signed prefix"
    );
}

/// plan-43 non-goal: the marker is additive and orthogonal to the `--sign`
/// feature — an image with `signing_metadata` carries both the `LC_NOTE` and the
/// `__MFB`/`__sign` segment.
#[test]
fn provenance_note_coexists_with_the_mfb_sign_segment() {
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![text_symbol("_main", 0)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: Some(br#"{"owner":"alice"}"#.to_vec()),
        rpaths: Vec::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "noted-signed", &image).expect("link signed mach-o");
    let bytes = std::fs::read(path).unwrap();
    let (offset, descriptor) = mfb_note(&bytes).expect("LC_NOTE owned by MFBasic");
    assert_eq!(descriptor, mfb_note_descriptor());
    assert!(offset + descriptor.len() <= code_signature_offset(&bytes));
    assert!(bytes
        .windows(b"__sign".len())
        .any(|window| window == b"__sign"));
    assert!(bytes
        .windows(br#"{"owner":"alice"}"#.len())
        .any(|window| window == br#"{"owner":"alice"}"#));
}

/// plan-43 acceptance: the marker must not invalidate the ad-hoc code signature
/// or change runtime behavior. `codesign -v` passing proves the `LC_NOTE` payload
/// lies inside `codeLimit` (a payload past it, or an `LC_NOTE` the two-pass sign
/// settle sized inconsistently, breaks verification); exit 7 proves dyld still
/// loads the image and reaches `_main`. Both the plain and the `--sign`
/// (`__MFB`/`__sign`) shapes, since the marker is orthogonal to that feature.
#[cfg(target_os = "macos")]
#[test]
fn noted_mach_o_verifies_and_runs() {
    let words: [u32; 3] = [
        0xD280_00E0, // _main: movz x0, #7
        0xD280_0030, //        movz x16, #1  (SYS_exit)
        0xD400_1001, //        svc  #0x80    -> exit(7)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    for (name, metadata) in [
        ("noted_run", None),
        ("noted_run_signed", Some(br#"{"owner":"ada"}"#.to_vec())),
    ] {
        let image = EncodedImage {
            text: text.clone(),
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![text_symbol("_main", 0)],
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: metadata,
            rpaths: Vec::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = write_executable(dir.path(), name, &image).expect("link mach-o");
        let verified = std::process::Command::new("/usr/bin/codesign")
            .arg("-v")
            .arg(&path)
            .status()
            .expect("run codesign");
        assert!(
            verified.success(),
            "{name}: codesign -v must still pass with the LC_NOTE present"
        );
        let status = std::process::Command::new(&path)
            .status()
            .expect("run binary");
        assert_eq!(
            status.code(),
            Some(7),
            "{name}: the marker is inert — _main must still run"
        );
    }
}

// Drives the multi-library Mach-O path (plan-linker.md §7) end to end against
// the real `tls` driver library, Network.framework: a hand-built program that
// imports a symbol from Network (ordinal 2) and `exit` from libSystem
// (ordinal 1), then links and executes. A wrong dylib ordinal or a missing
// LC_LOAD_DYLIB makes dyld fail to bind at launch, so a clean exit proves the
// generalization.
#[cfg(target_os = "macos")]
#[test]
fn links_and_runs_program_importing_from_two_dylibs() {
    // _main: x0 = nw_path_monitor_create(); exit(x0 != null ? 0 : 7).
    let words: [u32; 6] = [
        0x9400_0000, // bl _nw_path_monitor_create  (external branch26, patched)
        0xB400_0060, // cbz x0, fail (+12)
        0xD280_0000, // movz x0, #0
        0x1400_0002, // b done (+8)
        0xD280_00E0, // fail: movz x0, #7
        0x9400_0000, // done: bl _exit              (external branch26, patched)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![
            EncodedRelocation {
                offset: 0,
                target: "_nw_path_monitor_create".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("Network".to_string()),
            },
            EncodedRelocation {
                offset: 20,
                target: "_exit".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
        ],
        imports: vec![
            import("libSystem", "_exit"),
            import("Network", "_nw_path_monitor_create"),
        ],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };

    let dir = std::env::temp_dir().join(format!("mfb_nwlink_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = write_executable(&dir, "nwlink", &image).expect("link multi-dylib executable");
    let status = std::process::Command::new(&path)
        .status()
        .expect("run multi-dylib executable");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "program importing from libSystem + Network.framework should exit 0"
    );
}

fn text_symbol(name: &str, offset: usize) -> EncodedSymbol {
    EncodedSymbol {
        name: name.to_string(),
        section: EncodedSection::Text,
        offset,
    }
}

fn data_symbol(name: &str, offset: usize) -> EncodedSymbol {
    EncodedSymbol {
        name: name.to_string(),
        section: EncodedSection::Data,
        offset,
    }
}

// Drives every internal/data relocation arm of `patch_relocations` on a
// hand-built text blob: internal branch26, data page21/pageoff12. Asserts the
// opcode bits land where the AArch64 encoding expects.
#[test]
fn patches_internal_and_data_relocations() {
    let mut text = vec![
        0x00, 0x00, 0x00, 0x94, // bl _target       (internal branch26)
        0x00, 0x00, 0x00, 0x90, // adrp x0, _g      (data page21)
        0x00, 0x00, 0x00, 0x91, // add x0,x0,#off   (data pageoff12)
    ];
    let image = EncodedImage {
        text: text.clone(),
        data: vec![0; 8],
        rodata_size: 0,
        symbols: vec![
            text_symbol("_main", 0),
            text_symbol("_target", 8),
            data_symbol("_g", 0),
        ],
        relocations: vec![
            EncodedRelocation {
                offset: 0,
                target: "_target".to_string(),
                kind: "branch26".to_string(),
                binding: "internal".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 4,
                target: "_g".to_string(),
                kind: "page21".to_string(),
                binding: "data".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 8,
                target: "_g".to_string(),
                kind: "pageoff12".to_string(),
                binding: "data".to_string(),
                library: None,
            },
        ],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let text_vmaddr = VM_BASE + 0x4000;
    let data_vmaddr = text_vmaddr + 0x4000;
    patch_relocations(
        &mut text,
        &image,
        text_vmaddr,
        data_vmaddr,
        data_vmaddr,
        0,
        &ImportLocations::default(),
    )
    .expect("relocations");
    // The internal branch26 kept the bl opcode high byte (0x94) after patching.
    assert_eq!(text[3] & 0xfc, 0x94);
    // The adrp opcode (0x90 family) high bit set.
    assert_eq!(read_u32(&text, 4) & 0x9f00_0000, 0x9000_0000);
}

#[test]
fn patch_relocations_rejects_unsupported_kind() {
    let mut text = vec![0; 4];
    let image = EncodedImage {
        text: text.clone(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![text_symbol("_main", 0)],
        relocations: vec![EncodedRelocation {
            offset: 0,
            target: "_main".to_string(),
            kind: "bogus".to_string(),
            binding: "weird".to_string(),
            library: None,
        }],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let err = patch_relocations(
        &mut text,
        &image,
        VM_BASE,
        VM_BASE,
        VM_BASE,
        0,
        &ImportLocations::default(),
    )
    .expect_err("unsupported reloc");
    assert!(err.contains("does not support relocation"), "{err}");
}

#[test]
fn patch_relocations_rejects_unbound_external_symbols() {
    let make = |kind: &str| EncodedImage {
        text: vec![0; 4],
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![text_symbol("_main", 0)],
        relocations: vec![EncodedRelocation {
            offset: 0,
            target: "_unbound".to_string(),
            kind: kind.to_string(),
            binding: "external".to_string(),
            library: Some("libSystem".to_string()),
        }],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    for kind in ["branch26", "page21", "pageoff12"] {
        let image = make(kind);
        let mut text = image.text.clone();
        let err = patch_relocations(
            &mut text,
            &image,
            VM_BASE,
            VM_BASE,
            VM_BASE,
            0,
            &ImportLocations::default(),
        )
        .expect_err("unbound external");
        assert!(err.contains("cannot bind external"), "{kind}: {err}");
    }
}

// External page21/pageoff12 patched against a GOT entry (imported data global).
#[test]
fn patches_external_got_page_relocations() {
    let mut text = vec![
        0x00, 0x00, 0x00, 0x90, // adrp x0, GOT   (external page21)
        0x00, 0x00, 0x00, 0x91, // add  x0, ...   (external pageoff12)
    ];
    let image = EncodedImage {
        text: text.clone(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![text_symbol("_main", 0)],
        relocations: vec![
            EncodedRelocation {
                offset: 0,
                target: "environ".to_string(),
                kind: "page21".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
            EncodedRelocation {
                offset: 4,
                target: "environ".to_string(),
                kind: "pageoff12".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            },
        ],
        imports: vec![import("libSystem", "environ")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let mut locations = ImportLocations::default();
    locations
        .got_entries
        .insert("environ".to_string(), VM_BASE + 0x8000);
    patch_relocations(
        &mut text,
        &image,
        VM_BASE + 0x4000,
        VM_BASE + 0x4000,
        VM_BASE + 0x4000,
        0,
        &locations,
    )
    .expect("external got relocations");
    assert_eq!(read_u32(&text, 0) & 0x9f00_0000, 0x9000_0000);
    assert_eq!(read_u32(&text, 4) & 0xff00_0000, 0x9100_0000);
}

#[test]
fn symbol_vmaddr_rejects_unknown_symbol() {
    let image = EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let err =
        symbol_vmaddr(&image, "_nope", VM_BASE, VM_BASE, VM_BASE, 0).expect_err("unknown symbol");
    assert!(err.contains("does not resolve"), "{err}");
}

// A data symbol below `rodata_size` resolves into the read-only constant region
// (`rodata_vmaddr`); one at or above it into the writable `__DATA` (`data_vmaddr`),
// indexed past the constant prefix (bug-187).
#[test]
fn symbol_vmaddr_splits_constants_from_writable_data() {
    let image = EncodedImage {
        text: Vec::new(),
        data: vec![0; 0x2000],
        rodata_size: 0x1000,
        symbols: vec![data_symbol("_const", 0x40), data_symbol("_arena", 0x1000)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let text_vmaddr = VM_BASE;
    let rodata_vmaddr = VM_BASE + 0x4000;
    let data_vmaddr = VM_BASE + 0x8000;
    // Constant: `rodata_vmaddr + offset`.
    assert_eq!(
        symbol_vmaddr(
            &image,
            "_const",
            text_vmaddr,
            rodata_vmaddr,
            data_vmaddr,
            0x1000
        )
        .unwrap(),
        rodata_vmaddr + 0x40,
    );
    // Writable: `data_vmaddr + (offset - rodata_size)` — the first writable byte
    // sits at the base of `__DATA`.
    assert_eq!(
        symbol_vmaddr(
            &image,
            "_arena",
            text_vmaddr,
            rodata_vmaddr,
            data_vmaddr,
            0x1000
        )
        .unwrap(),
        data_vmaddr,
    );
}

#[test]
fn dylib_path_covers_all_libraries_and_rejects_unknown() {
    assert_eq!(
        dylib_path("libSystem").unwrap(),
        "/usr/lib/libSystem.B.dylib"
    );
    assert_eq!(
        dylib_path("Network").unwrap(),
        "/System/Library/Frameworks/Network.framework/Network"
    );
    assert_eq!(dylib_path("libz").unwrap(), "/usr/lib/libz.1.dylib");
    assert!(dylib_path("Unknown")
        .expect_err("unknown lib")
        .contains("no dylib path"));
}

#[test]
fn library_ordinal_rejects_absent_library() {
    let libraries = import_libraries(&EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: vec![import("libSystem", "_exit")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    })
    .unwrap();
    assert_eq!(library_ordinal(&libraries, "libSystem").unwrap(), 1);
    assert!(library_ordinal(&libraries, "Network")
        .expect_err("absent library")
        .contains("no dylib ordinal"));
}

#[test]
fn data_const_helpers_size_by_slots() {
    let none = EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        rodata_size: 0,
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    assert_eq!(data_const_size(&none), 0);
    assert_eq!(data_const_section_count(0, 0, false), 0);
    assert_eq!(data_const_section_count(1, 0, false), 1);
    assert_eq!(data_const_section_count(0, 1, false), 1);
    assert_eq!(data_const_section_count(2, 3, false), 2);
    // The read-only `__const` block adds a third section (bug-187).
    assert_eq!(data_const_section_count(0, 0, true), 1);
    assert_eq!(data_const_section_count(2, 3, true), 3);
    let some = EncodedImage {
        imports: vec![import("libSystem", "_exit")],
        initializers: vec!["_init".to_string()],
        ..none
    };
    // Two slots (one import + one initializer) round up to a page.
    assert_eq!(data_const_size(&some), PAGE_SIZE);
}

#[test]
fn plist_escape_handles_all_predefined_entities() {
    assert_eq!(
        plist_escape("a&b<c>d\"e'f"),
        "a&amp;b&lt;c&gt;d&quot;e&apos;f"
    );
}

#[test]
fn rejects_entry_symbol_not_in_text() {
    // The entry names a data symbol → encoding must refuse.
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        data: vec![0; 4],
        rodata_size: 0,
        symbols: vec![data_symbol("_main", 0)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(dir.path(), "bad", &image).expect_err("entry not text");
    assert!(err.contains("does not resolve to text"), "{err}");
}

#[test]
fn dylib_path_resolves_app_mode_frameworks() {
    // App mode (plan-04-macos-app.md §6.5) binds against these frameworks.
    assert_eq!(
        dylib_path("AppKit").unwrap(),
        "/System/Library/Frameworks/AppKit.framework/AppKit"
    );
    assert_eq!(
        dylib_path("Foundation").unwrap(),
        "/System/Library/Frameworks/Foundation.framework/Foundation"
    );
    assert_eq!(dylib_path("libobjc").unwrap(), "/usr/lib/libobjc.A.dylib");
}

#[test]
fn app_info_plist_has_required_bundle_keys() {
    let plist = app_info_plist("hello", "0.1.0");
    assert!(plist.contains("<key>CFBundleExecutable</key>\n  <string>hello</string>"));
    assert!(plist.contains("<key>CFBundleName</key>\n  <string>hello</string>"));
    assert!(plist.contains("<string>dev.mfbasic.hello</string>"));
    assert!(plist.contains("<key>CFBundlePackageType</key>\n  <string>APPL</string>"));
    assert!(plist.contains("<key>NSPrincipalClass</key>\n  <string>NSApplication</string>"));
}

// bug-248: App Store upload validation (`altool`) rejects a bundle whose
// Info.plist omits CFBundleVersion or CFBundleShortVersionString. Both carry the
// manifest `version`.
#[test]
fn app_info_plist_publishes_manifest_version() {
    let plist = app_info_plist("hello", "0.1.0");
    assert!(plist.contains("<key>CFBundleShortVersionString</key>\n  <string>0.1.0</string>"));
    assert!(plist.contains("<key>CFBundleVersion</key>\n  <string>0.1.0</string>"));
}

#[test]
fn app_info_plist_escapes_xml_metacharacters() {
    let plist = app_info_plist("a<b&c", "1.0");
    assert!(plist.contains("<string>a&lt;b&amp;c</string>"));
    assert!(!plist.contains("a<b&c"));
}

#[test]
fn app_info_plist_escapes_xml_metacharacters_in_version() {
    let plist = app_info_plist("hello", "1.0<beta&2");
    assert!(plist.contains("<key>CFBundleVersion</key>\n  <string>1.0&lt;beta&amp;2</string>"));
    assert!(!plist.contains("1.0<beta&2"));
}

// End-to-end Phase 2 (plan-04-macos-app.md §5.2): a hand-built program that
// exits 0 is written as a `.app` bundle. Asserts the bundle layout, that the
// inner Mach-O is byte-identical to the console `<name>.out`, that Info.plist
// is present, and that launching `Contents/MacOS/<name>` runs and exits 0.
#[cfg(target_os = "macos")]
#[test]
fn writes_and_launches_app_bundle() {
    let words: [u32; 3] = [
        0xD280_0000, // movz x0, #0
        0xD280_0030, // movz x16, #1   (SYS_exit)
        0xD400_1001, // svc  #0x80      -> exit(0)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!("mfb_appbundle_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let bundle =
        write_app_bundle(&dir, "windowed", &image, None, "0.1.0").expect("write app bundle");
    assert_eq!(bundle, dir.join("build").join("windowed.app"));
    let exe = bundle.join("Contents/MacOS/windowed");
    let plist = bundle.join("Contents/Info.plist");
    let icns = bundle.join("Contents/Resources/AppIcon.icns");
    assert!(exe.is_file(), "bundle executable must exist");
    assert!(plist.is_file(), "Info.plist must exist");
    assert!(icns.is_file(), "AppIcon.icns must exist");
    assert_eq!(
        &std::fs::read(&icns).unwrap()[0..4],
        b"icns",
        "AppIcon.icns must begin with the icns magic"
    );

    // The bundled binary must be byte-identical to the console `.out` — for an
    // image that vendors nothing, which is this one and every existing project.
    // A vendoring image adds one `LC_RPATH` whose string is per output shape, so
    // the two genuinely differ there; that narrowed invariant is pinned by
    // `a_vendoring_bundle_differs_from_the_console_binary_by_exactly_the_rpath`.
    let out = write_executable(&dir, "windowed", &image).expect("write console executable");
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        std::fs::read(&out).unwrap(),
        "bundled Mach-O must match the console executable bytes"
    );

    let status = std::process::Command::new(&exe)
        .status()
        .expect("run bundled executable");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "launching the bundled executable should exit 0"
    );
}

// Phase 2 framework-import proof for the libraries app mode needs
// (plan-04-macos-app.md §6.5): a hand-built `_main` calls
// `objc_getClass("NSObject")` (libobjc) and exits 0, packaged as a `.app`.
// libobjc binding is exercised through the GOT, a data relocation resolves
// the class-name string, and a clean exit proves the LC_LOAD_DYLIB for
// libobjc resolves at launch. NSObject is registered by the Objective-C
// runtime itself, so this needs no Foundation/AppKit at run time.
#[cfg(target_os = "macos")]
#[test]
fn links_and_launches_app_bundle_importing_libobjc() {
    // _main:
    //   adrp x0, name ; add x0, x0, pageoff(name)   -> x0 = "NSObject"
    //   bl _objc_getClass
    //   movz x0, #0 ; movz x16, #1 ; svc #0x80       -> exit(0)
    let words: [u32; 6] = [
        0x9000_0000, // adrp x0, name        (data page21, patched)
        0x9100_0000, // add  x0, x0, #pageoff (data pageoff12, patched)
        0x9400_0000, // bl _objc_getClass     (external branch26, patched)
        0xD280_0000, // movz x0, #0
        0xD280_0030, // movz x16, #1
        0xD400_1001, // svc  #0x80            -> exit(0)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    let mut data = b"NSObject".to_vec();
    data.push(0);
    let image = EncodedImage {
        text,
        data,
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_class_name".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: vec![
            EncodedRelocation {
                offset: 0,
                target: "_class_name".to_string(),
                kind: "page21".to_string(),
                binding: "data".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 4,
                target: "_class_name".to_string(),
                kind: "pageoff12".to_string(),
                binding: "data".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 8,
                target: "_objc_getClass".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libobjc".to_string()),
            },
        ],
        imports: vec![import("libobjc", "_objc_getClass")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!("mfb_objclink_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let bundle =
        write_app_bundle(&dir, "objcapp", &image, None, "0.1.0").expect("write libobjc app bundle");
    let exe = bundle.join("Contents/MacOS/objcapp");
    let status = std::process::Command::new(&exe)
        .status()
        .expect("run libobjc app bundle");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "app bundle importing libobjc should bind and exit 0"
    );
}

// Build a real AArch64 image through the arch encoder so text and relocations are
// self-consistent, then drive the *full* Mach-O byte writer end to end without
// running the binary. This is host-neutral (pure byte generation + a tempfile
// write) so it covers the imports + data + initializer path — data_const_segment,
// data_segment, dyld_info, bind_info, symbol_table, string_table, load_dylib,
// dysymtab, rebase_info, linkedit_layout — on Linux CI too, where the
// `#[cfg(target_os = "macos")]` launch tests do not run.
fn encode_aarch64(
    imports: Vec<crate::target::shared::code::CodeImport>,
    data_objects: Vec<crate::target::shared::code::CodeDataObject>,
    functions: Vec<crate::target::shared::code::CodeFunction>,
    entry: &str,
) -> EncodedImage {
    let plan = crate::target::shared::code::NativeCodePlan {
        target: "macos-aarch64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "aarch64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some(entry.to_string()),
        imports,
        data_objects,
        functions,
    };
    crate::arch::aarch64::encode::encode(&plan).expect("aarch64 encode")
}

fn code_fn(
    name: &str,
    instructions: Vec<crate::target::shared::code::CodeInstruction>,
) -> crate::target::shared::code::CodeFunction {
    crate::target::shared::code::CodeFunction {
        name: name.to_string(),
        symbol: name.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: crate::target::shared::code::CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

fn inst(op: &str, fields: &[(&'static str, &str)]) -> crate::target::shared::code::CodeInstruction {
    let mut instruction = crate::target::shared::code::CodeInstruction::new(op);
    for (key, value) in fields {
        instruction = instruction.field(key, value);
    }
    instruction
}

#[test]
fn writes_full_mach_o_with_imports_data_and_initializer() {
    let main = code_fn(
        "_main",
        vec![
            inst("adrp", &[("dst", "x0"), ("symbol", "_msg")]),
            inst(
                "add_pageoff",
                &[("dst", "x0"), ("src", "x0"), ("symbol", "_msg")],
            ),
            inst("bl", &[("target", "_write")]),
            inst("ret", &[]),
        ],
    );
    let init0 = code_fn("_init0", vec![inst("ret", &[])]);
    let mut image = encode_aarch64(
        vec![crate::target::shared::code::CodeImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
        }],
        vec![crate::target::shared::code::CodeDataObject {
            symbol: "_msg".to_string(),
            kind: "string".to_string(),
            layout: String::new(),
            align: 8,
            size: 16,
            value: "hi".to_string(),
        }],
        vec![main, init0],
        "_main",
    );
    // A load-time initializer forces the __DATA_CONST + __mod_init_func + rebase path.
    image.initializers = vec!["_init0".to_string()];

    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "full", &image).expect("write full mach-o");
    let bytes = std::fs::read(&path).unwrap();
    // Mach-O 64 magic (MH_MAGIC_64, little-endian on disk).
    assert_eq!(&bytes[..4], &[0xCF, 0xFA, 0xED, 0xFE]);
    // The dylib install path (LC_LOAD_DYLIB) and the imported symbol name (string
    // table) are both present, proving the import path emitted its load commands.
    assert!(bytes
        .windows(b"/usr/lib/libSystem.B.dylib".len())
        .any(|window| window == b"/usr/lib/libSystem.B.dylib"));
    assert!(bytes
        .windows(b"_write".len())
        .any(|window| window == b"_write"));
    // The string data lands in the writable __DATA segment.
    assert!(bytes
        .windows(b"__DATA".len())
        .any(|window| window == b"__DATA"));
    assert!(bytes.windows(2).any(|window| window == b"hi"));
    // The file is comfortably larger than a page (code + data-const + data + linkedit).
    assert!(bytes.len() > PAGE_SIZE);
}

// `write_app_bundle` host-neutral: assert the on-disk `.app` layout, the
// Info.plist contents, and that the inner Mach-O carries the right magic, without
// launching anything (the launch proof stays behind `#[cfg(target_os = "macos")]`).
#[test]
fn write_app_bundle_creates_layout_and_plist_host_neutral() {
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6], // ret
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![text_symbol("_main", 0)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let bundle =
        write_app_bundle(dir.path(), "demo", &image, None, "2.3.4").expect("write app bundle");
    assert_eq!(bundle, dir.path().join("build").join("demo.app"));

    let exe = bundle.join("Contents/MacOS/demo");
    let plist = bundle.join("Contents/Info.plist");
    let icns = bundle.join("Contents/Resources/AppIcon.icns");
    assert!(exe.is_file(), "bundle executable must exist");
    assert!(plist.is_file(), "Info.plist must exist");
    assert!(icns.is_file(), "AppIcon.icns must exist");
    assert_eq!(
        &std::fs::read(&icns).unwrap()[0..4],
        b"icns",
        "AppIcon.icns must begin with the icns magic"
    );

    let plist_text = std::fs::read_to_string(&plist).unwrap();
    assert!(plist_text.contains("<key>CFBundleExecutable</key>\n  <string>demo</string>"));
    assert!(plist_text.contains("dev.mfbasic.demo"));
    assert!(plist_text.contains("<key>CFBundleIconFile</key>\n  <string>AppIcon</string>"));

    let exe_bytes = std::fs::read(&exe).unwrap();
    assert_eq!(&exe_bytes[..4], &[0xCF, 0xFA, 0xED, 0xFE]);
}

// Directly exercise the small layout helpers across the four presence
// combinations, so their branches are covered without a full write.
#[test]
fn code_offset_and_layout_helpers_vary_with_presence() {
    let no_libs: [(String, String); 0] = [];
    let libs = [(
        "libSystem".to_string(),
        "/usr/lib/libSystem.B.dylib".to_string(),
    )];
    let no_rpaths: [String; 0] = [];
    let rpaths = ["@loader_path/vendor".to_string()];
    // A data-const/dylib image needs a larger header than a bare one.
    let bare = super::macho::code_offset(&no_libs, &no_rpaths, false, false, false, false);
    let with_libs = super::macho::code_offset(&libs, &no_rpaths, false, false, false, false);
    let with_data = super::macho::code_offset(&no_libs, &no_rpaths, false, false, true, false);
    let with_sign = super::macho::code_offset(&no_libs, &no_rpaths, true, false, false, false);
    let with_rodata = super::macho::code_offset(&no_libs, &no_rpaths, false, false, false, true);
    // plan-46-D §4.3: an LC_RPATH grows the header by exactly its command size,
    // and `load_commands_size` must agree with what the emitter writes or dyld
    // rejects the image.
    let with_rpath = super::macho::code_offset(&no_libs, &rpaths, false, false, false, false);
    assert!(with_libs > bare, "dylib load commands grow the header");
    assert!(with_data > bare, "the __DATA segment grows the header");
    assert!(with_sign > bare, "the signing segment grows the header");
    assert!(
        with_rodata > bare,
        "the read-only __DATA_CONST,__const block grows the header"
    );
    assert!(with_rpath > bare, "an LC_RPATH grows the header");
    assert_eq!(
        with_rpath - bare,
        super::commands::rpath_command_size("@loader_path/vendor"),
        "the header must grow by exactly one rpath command size"
    );
    // All are 4-byte aligned (the code offset rounds to 4).
    for offset in [
        bare,
        with_libs,
        with_data,
        with_sign,
        with_rodata,
        with_rpath,
    ] {
        assert_eq!(offset % 4, 0);
    }
    // macho_layout: with writable data the __DATA segment is page-aligned and
    // non-zero.
    let layout = super::macho::macho_layout(bare, 16, 32, 0, 0, 0);
    assert_eq!(layout.data_seg_size, PAGE_SIZE);
    assert!(layout.data_seg_file_offset.is_multiple_of(PAGE_SIZE));
    // No data → no __DATA segment.
    let empty = super::macho::macho_layout(bare, 16, 0, 0, 0, 0);
    assert_eq!(empty.data_seg_size, 0);
    // All-constant data → no writable __DATA segment (it rides in __DATA_CONST).
    let all_rodata = super::macho::macho_layout(bare, 16, 32, 32, 0, 0);
    assert_eq!(all_rodata.data_seg_size, 0);
}

/// A hand-built image that exits 0, used by the `LC_RPATH` tests below.
#[cfg(test)]
fn exit_zero_image(rpaths: Vec<String>) -> EncodedImage {
    let words: [u32; 3] = [
        0xD280_0000, // movz x0, #0
        0xD280_0030, // movz x16, #1   (SYS_exit)
        0xD400_1001, // svc  #0x80      -> exit(0)
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths,
    }
}

/// Decode `(ncmds, sizeofcmds)` and every `LC_RPATH` path from a Mach-O header.
///
/// Reads the load commands the way `dyld` does, so the assertions below catch a
/// `sizeofcmds`/`ncmds` that disagrees with the bytes actually emitted — the
/// plan-46-D §2.2 triple-maintenance hazard, which is invisible to a round-trip
/// test and fatal at exec.
#[cfg(test)]
fn mach_o_rpaths(bytes: &[u8]) -> (u32, u32, Vec<String>) {
    let ncmds = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let sizeofcmds = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    let mut rpaths = Vec::new();
    let mut offset = 32usize;
    for _ in 0..ncmds {
        let cmd = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let cmdsize =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if cmd == 0x8000_001c {
            let path_offset =
                u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
            let start = offset + path_offset;
            let end = start
                + bytes[start..offset + cmdsize]
                    .iter()
                    .position(|byte| *byte == 0)
                    .expect("NUL-terminated rpath");
            rpaths.push(String::from_utf8(bytes[start..end].to_vec()).expect("utf-8 rpath"));
        }
        offset += cmdsize;
    }
    // The walk must land exactly on the declared end of the load commands.
    ((offset - 32) as u32, sizeofcmds, rpaths)
}

// plan-46-D §4.3: an image that vendors nothing carries no LC_RPATH at all, so
// every existing binary stays byte-identical.
#[test]
fn a_non_vendor_image_emits_no_rpath() {
    let image = exit_zero_image(Vec::new());
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "norpath", &image).expect("link");
    let bytes = std::fs::read(&path).unwrap();
    let (walked, declared, rpaths) = mach_o_rpaths(&bytes);
    assert!(rpaths.is_empty(), "no vendor libraries → no LC_RPATH");
    assert_eq!(walked, declared, "sizeofcmds must match the emitted bytes");
}

// The triple-maintenance hazard (§2.2): emission, `load_commands_size`, and
// `load_command_count` are three independent computations feeding `sizeofcmds`
// and `ncmds`. If they disagree, dyld rejects the image at launch — so assert the
// header math AND actually launch it.
#[test]
fn a_vendoring_image_emits_one_rpath_with_a_consistent_header() {
    let image = exit_zero_image(vec!["@loader_path/vendor".to_string()]);
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "rpath", &image).expect("link");
    let bytes = std::fs::read(&path).unwrap();
    let (walked, declared, rpaths) = mach_o_rpaths(&bytes);
    assert_eq!(rpaths, vec!["@loader_path/vendor".to_string()]);
    assert_eq!(
        walked, declared,
        "sizeofcmds must match the bytes actually emitted, or dyld rejects the image"
    );
}

// The console and `.app` shapes need *different* rpath strings, so the app one is
// asserted separately.
#[test]
fn an_app_bundle_rpath_points_at_the_frameworks_directory() {
    let image = exit_zero_image(vec!["@executable_path/../Frameworks".to_string()]);
    let dir = tempfile::tempdir().unwrap();
    let bundle =
        write_app_bundle(dir.path(), "rpathapp", &image, None, "0.1.0").expect("write bundle");
    let bytes = std::fs::read(bundle.join("Contents/MacOS/rpathapp")).unwrap();
    let (walked, declared, rpaths) = mach_o_rpaths(&bytes);
    assert_eq!(rpaths, vec!["@executable_path/../Frameworks".to_string()]);
    assert_eq!(walked, declared);
}

// plan-46-D §4.4: the bundled Mach-O is byte-identical to the console `.out` for
// an image that vendors nothing, and differs by **exactly** the one LC_RPATH when
// it vendors. The narrowed invariant is pinned here rather than left implicit.
#[test]
fn a_vendoring_bundle_differs_from_the_console_binary_by_exactly_the_rpath() {
    let dir = tempfile::tempdir().unwrap();

    // No vendor libraries → identical bytes (the unqualified invariant still
    // holds for every existing project).
    let plain = exit_zero_image(Vec::new());
    let console = write_executable(dir.path(), "same", &plain).expect("console");
    let bundle = write_app_bundle(dir.path(), "same", &plain, None, "0.1.0").expect("bundle");
    assert_eq!(
        std::fs::read(&console).unwrap(),
        std::fs::read(bundle.join("Contents/MacOS/same")).unwrap(),
        "with no vendor libraries the two shapes must stay byte-identical"
    );

    // Vendor libraries → each shape carries its own rpath, and that is the only
    // difference. They load from genuinely different places, so identical bytes
    // would mean one of them is wrong.
    let console_image = exit_zero_image(vec!["@loader_path/vendor".to_string()]);
    let app_image = exit_zero_image(vec!["@executable_path/../Frameworks".to_string()]);
    let console_v = write_executable(dir.path(), "diff", &console_image).expect("console");
    let bundle_v = write_app_bundle(dir.path(), "diff", &app_image, None, "0.1.0").expect("bundle");
    let console_bytes = std::fs::read(&console_v).unwrap();
    let app_bytes = std::fs::read(bundle_v.join("Contents/MacOS/diff")).unwrap();
    assert_ne!(console_bytes, app_bytes);
    let (_, _, console_rpaths) = mach_o_rpaths(&console_bytes);
    let (_, _, app_rpaths) = mach_o_rpaths(&app_bytes);
    assert_eq!(console_rpaths, vec!["@loader_path/vendor".to_string()]);
    assert_eq!(
        app_rpaths,
        vec!["@executable_path/../Frameworks".to_string()]
    );
}

// The header math is only proven by a real launch: an `LC_RPATH` shifts every
// subsequent offset, including the LC_CODE_SIGNATURE arm64 macOS requires
// (`.ai/compiler.md` runtime completion gate).
#[cfg(target_os = "macos")]
#[test]
fn a_vendoring_binary_still_launches() {
    let image = exit_zero_image(vec!["@loader_path/vendor".to_string()]);
    let dir = std::env::temp_dir().join(format!("mfb_rpath_launch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = write_executable(&dir, "launch", &image).expect("link");
    let status = std::process::Command::new(&path)
        .status()
        .expect("run rpath executable");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "an image carrying an LC_RPATH must still launch: dyld rejects a header \
         whose sizeofcmds/ncmds disagree with its contents"
    );
}
