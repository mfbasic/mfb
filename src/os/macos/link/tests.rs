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
    };
    let text_vmaddr = VM_BASE + 0x4000;
    let locations =
        append_import_stubs(&mut text, &image, text_vmaddr, 0x4000, 0).expect("import stubs");

    let data_vmaddr = text_vmaddr + text.len() as u64;
    patch_relocations(&mut text, &image, text_vmaddr, data_vmaddr, &locations)
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
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: vec![import("lib16", "_late")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
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
    };
    let text_vmaddr = VM_BASE + 0x4000;
    let data_vmaddr = text_vmaddr + 0x4000;
    patch_relocations(
        &mut text,
        &image,
        text_vmaddr,
        data_vmaddr,
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
    };
    let err = patch_relocations(
        &mut text,
        &image,
        VM_BASE,
        VM_BASE,
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
    };
    for kind in ["branch26", "page21", "pageoff12"] {
        let image = make(kind);
        let mut text = image.text.clone();
        let err = patch_relocations(
            &mut text,
            &image,
            VM_BASE,
            VM_BASE,
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
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let err = symbol_vmaddr(&image, "_nope", VM_BASE, VM_BASE).expect_err("unknown symbol");
    assert!(err.contains("does not resolve"), "{err}");
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
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: vec![import("libSystem", "_exit")],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
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
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    assert_eq!(data_const_size(&none), 0);
    assert_eq!(data_const_section_count(0, 0), 0);
    assert_eq!(data_const_section_count(1, 0), 1);
    assert_eq!(data_const_section_count(0, 1), 1);
    assert_eq!(data_const_section_count(2, 3), 2);
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
        symbols: vec![data_symbol("_main", 0)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
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
    let plist = app_info_plist("hello");
    assert!(plist.contains("<key>CFBundleExecutable</key>\n  <string>hello</string>"));
    assert!(plist.contains("<key>CFBundleName</key>\n  <string>hello</string>"));
    assert!(plist.contains("<string>dev.mfbasic.hello</string>"));
    assert!(plist.contains("<key>CFBundlePackageType</key>\n  <string>APPL</string>"));
    assert!(plist.contains("<key>NSPrincipalClass</key>\n  <string>NSApplication</string>"));
}

#[test]
fn app_info_plist_escapes_xml_metacharacters() {
    let plist = app_info_plist("a<b&c");
    assert!(plist.contains("<string>a&lt;b&amp;c</string>"));
    assert!(!plist.contains("a<b&c"));
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
    };
    let dir = std::env::temp_dir().join(format!("mfb_appbundle_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let bundle = write_app_bundle(&dir, "windowed", &image).expect("write app bundle");
    assert_eq!(bundle, dir.join("windowed.app"));
    let exe = bundle.join("Contents/MacOS/windowed");
    let plist = bundle.join("Contents/Info.plist");
    assert!(exe.is_file(), "bundle executable must exist");
    assert!(plist.is_file(), "Info.plist must exist");

    // The bundled binary must be byte-identical to the console `.out`.
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
    };
    let dir = std::env::temp_dir().join(format!("mfb_objclink_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let bundle = write_app_bundle(&dir, "objcapp", &image).expect("write libobjc app bundle");
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
