use super::commands::bind_info;
use super::*;
use crate::arch::aarch64::encode::{
    EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind,
};

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
    let path = write_executable(&dir, "modinitimp", &image)
        .expect("link initializer+import executable");
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
