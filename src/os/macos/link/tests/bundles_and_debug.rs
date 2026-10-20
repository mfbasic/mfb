//! plan-176-J: the Mach-O linker's bundles_and_debug tests. The shared image builders live in\n//! `link/tests/mod.rs`.

use super::*;

#[test]
fn app_info_plist_has_required_bundle_keys() {
    let plist = app_info_plist("hello", "0.1.0");
    assert!(plist.contains("<key>CFBundleExecutable</key>\n  <string>hello</string>"));
    assert!(plist.contains("<key>CFBundleName</key>\n  <string>hello</string>"));
    assert!(plist.contains("<string>dev.mfbasic.hello</string>"));
    assert!(plist.contains("<key>CFBundlePackageType</key>\n  <string>APPL</string>"));
    assert!(plist.contains("<key>NSPrincipalClass</key>\n  <string>NSApplication</string>"));
    // plan-157-C: the Documents-folder privacy prompt carries a purpose string.
    assert!(plist.contains(
        "<key>NSDocumentsFolderUsageDescription</key>\n  \
         <string>hello reads and writes files in your Documents folder.</string>"
    ));
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
// ===== App-mode bundle (.app) end-to-end =====
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
        ..none()
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
        ..none()
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
        vec![crate::codegen::engine::types::CodeImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
        }],
        vec![crate::codegen::engine::types::CodeDataObject {
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
        ..none()
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
        debug_symbols: false,
    }
}

/// Decode `(ncmds, sizeofcmds)` and every `LC_RPATH` path from a Mach-O header.
///
/// Reads the load commands the way `dyld` does, so the assertions below catch a
/// `sizeofcmds`/`ncmds` that disagrees with the bytes actually emitted — the
/// plan-46-D §2.2 triple-maintenance hazard, which is invisible to a round-trip
/// test and fatal at exec.
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
// ===== RPATH / vendoring =====

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

/// bug-503: the project name is `Path::join`ed into `build/<name>.out`, so a
/// manifest name carrying a path separator or `..` must be refused before any
/// byte is written — never resolved to a path outside `build/`.
#[test]
fn refuses_to_write_an_executable_under_a_traversing_name() {
    let mut text = Vec::new();
    put_u32(&mut text, 0xd280_0000); // movz w0, #0
    let image = EncodedImage {
        text,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    // `build/../evil.out` resolves to `<dir>/evil.out`: outside the build dir.
    let error = write_executable(dir.path(), "../evil", &image)
        .expect_err("a traversing project name must be rejected");
    assert!(error.contains("not a valid path component"), "{error}");
    assert!(
        !dir.path().join("evil.out").exists(),
        "an executable escaped build/ under a traversing name"
    );
    let error = write_app_bundle(dir.path(), "../evil", &image, None, "1.0.0")
        .expect_err("a traversing project name must be rejected by the bundle writer");
    assert!(error.contains("not a valid path component"), "{error}");
    assert!(!dir.path().join("evil.app").exists());
    // A leading `.` hides the artifact; an absolute name would ignore `build/`.
    for name in [".hidden", "/tmp/evil", "a/b"] {
        assert!(
            write_executable(dir.path(), name, &image).is_err(),
            "{name}"
        );
    }
    assert!(
        !dir.path().join("build").exists(),
        "nothing may be created on refusal"
    );
}

// ===== plan-166: `--debug` symbol table =====

/// The `LC_SYMTAB` / `LC_DYSYMTAB` / `LC_FUNCTION_STARTS` view of a Mach-O, read
/// the way `nm` and dyld do: from the load commands, not from the writer's
/// layout structs.
struct ParsedSymtab {
    /// `(name, n_type, n_sect, n_value)` in table order.
    symbols: Vec<(String, u8, u8, u64)>,
    /// `ilocalsym, nlocalsym, iextdefsym, nextdefsym, iundefsym, nundefsym`.
    dysymtab: [u32; 6],
    indirect: Vec<u32>,
    /// Function start file offsets, decoded from the ULEB deltas.
    function_starts: Vec<u64>,
    /// `(sectname, addr)` in ordinal order (index 0 = ordinal 1).
    sections: Vec<(String, u64)>,
}

fn parse_symtab(bytes: &[u8]) -> ParsedSymtab {
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let fixed = |at: usize| {
        let raw = &bytes[at..at + 16];
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(16);
        String::from_utf8(raw[..end].to_vec()).unwrap()
    };
    let mut parsed = ParsedSymtab {
        symbols: Vec::new(),
        dysymtab: [0; 6],
        indirect: Vec::new(),
        function_starts: Vec::new(),
        sections: Vec::new(),
    };
    let mut command = 32;
    for _ in 0..u32_at(16) {
        match u32_at(command) {
            0x19 => {
                for index in 0..u32_at(command + 64) as usize {
                    let section = command + 72 + index * 80;
                    parsed.sections.push((fixed(section), u64_at(section + 32)));
                }
            }
            0x2 => {
                let symoff = u32_at(command + 8) as usize;
                let nsyms = u32_at(command + 12) as usize;
                let stroff = u32_at(command + 16) as usize;
                for index in 0..nsyms {
                    let entry = symoff + index * 16;
                    let name_at = stroff + u32_at(entry) as usize;
                    let end = name_at + bytes[name_at..].iter().position(|b| *b == 0).unwrap();
                    parsed.symbols.push((
                        String::from_utf8(bytes[name_at..end].to_vec()).unwrap(),
                        bytes[entry + 4],
                        bytes[entry + 5],
                        u64_at(entry + 8),
                    ));
                }
            }
            0xb => {
                for (slot, value) in parsed.dysymtab.iter_mut().enumerate() {
                    *value = u32_at(command + 8 + slot * 4);
                }
                let indirectoff = u32_at(command + 56) as usize;
                let count = u32_at(command + 60) as usize;
                parsed.indirect = (0..count).map(|i| u32_at(indirectoff + i * 4)).collect();
            }
            0x26 => {
                let offset = u32_at(command + 8) as usize;
                let size = u32_at(command + 12) as usize;
                let mut cursor = offset;
                let mut address = 0u64;
                while cursor < offset + size {
                    let mut delta = 0u64;
                    let mut shift = 0;
                    loop {
                        let byte = bytes[cursor];
                        cursor += 1;
                        delta |= ((byte & 0x7f) as u64) << shift;
                        shift += 7;
                        if byte & 0x80 == 0 {
                            break;
                        }
                    }
                    if delta == 0 {
                        break;
                    }
                    address += delta;
                    parsed.function_starts.push(address);
                }
                assert!(
                    bytes[cursor..offset + size].iter().all(|b| *b == 0),
                    "LC_FUNCTION_STARTS pads with zeros after its terminator"
                );
            }
            _ => {}
        }
        command += u32_at(command + 4) as usize;
    }
    parsed
}

fn ordinal_of(parsed: &ParsedSymtab, name: &str) -> u8 {
    parsed
        .sections
        .iter()
        .position(|(sectname, _)| sectname == name)
        .unwrap_or_else(|| panic!("section {name} present")) as u8
        + 1
}

fn section_addr(parsed: &ParsedSymtab, name: &str) -> u64 {
    parsed.sections[ordinal_of(parsed, name) as usize - 1].1
}

/// Two functions, one constant and one writable data object, and one import:
/// every section kind a local can land in, plus the stub/GOT path.
fn debug_image() -> EncodedImage {
    let main = code_fn(
        "_main",
        vec![
            inst("adrp", &[("dst", "x0"), ("symbol", "_msg")]),
            inst(
                "add_pageoff",
                &[("dst", "x0"), ("src", "x0"), ("symbol", "_msg")],
            ),
            inst("bl", &[("target", "_helper")]),
            inst("bl", &[("target", "_write")]),
            inst("ret", &[]),
        ],
    );
    let helper = code_fn("_helper", vec![inst("ret", &[])]);
    let object = |symbol: &str, kind: &str| crate::codegen::engine::types::CodeDataObject {
        symbol: symbol.to_string(),
        kind: kind.to_string(),
        layout: String::new(),
        align: 8,
        size: 16,
        value: "hi".to_string(),
    };
    let mut image = encode_aarch64(
        vec![crate::codegen::engine::types::CodeImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
        }],
        vec![object("_msg", "constant"), object("_glob", "global")],
        vec![main, helper],
        "_main",
    );
    image.debug_symbols = true;
    image
}

#[test]
fn debug_symbols_lists_locals_before_imports() {
    let image = debug_image();
    assert!(image.rodata_size > 0 && image.data.len() > image.rodata_size);
    let parsed = parse_symtab(&encode_executable_bytes("dbg", &image).unwrap());
    let locals = &parsed.symbols[..4];
    let names: Vec<_> = locals.iter().map(|symbol| symbol.0.as_str()).collect();
    // Text first by offset, then data by offset (constants, then writable).
    assert_eq!(names, ["_main", "_helper", "_msg", "_glob"]);
    let text = section_addr(&parsed, "__text");
    let rodata = section_addr(&parsed, "__const");
    let data = section_addr(&parsed, "__data");
    for (name, n_type, n_sect, n_value) in locals {
        assert_eq!(*n_type, 0x0e, "{name}: N_SECT, not N_EXT");
        let expected_sect = match name.as_str() {
            "_main" | "_helper" => ordinal_of(&parsed, "__text"),
            "_msg" => ordinal_of(&parsed, "__const"),
            _ => ordinal_of(&parsed, "__data"),
        };
        assert_eq!(*n_sect, expected_sect, "{name}: n_sect");
        let expected =
            symbol_vmaddr(&image, name, text, data, Some((rodata, image.rodata_size))).unwrap();
        assert_eq!(*n_value, expected, "{name}: n_value");
    }
    assert_eq!(parsed.symbols[4], ("_write".to_string(), 0x1, 0, 0));
    assert_eq!(parsed.symbols.len(), 5);
}

#[test]
fn debug_symbols_shift_indirect_table_and_iundefsym() {
    let parsed = parse_symtab(&encode_executable_bytes("dbg", &debug_image()).unwrap());
    // ilocalsym, nlocalsym, iextdefsym, nextdefsym, iundefsym, nundefsym.
    assert_eq!(parsed.dysymtab, [0, 4, 4, 0, 4, 1]);
    // The one stub's indirect entry names `_write`, which now sits at index 4.
    assert_eq!(parsed.indirect, vec![4]);
    assert_eq!(parsed.symbols[parsed.indirect[0] as usize].0, "_write");

    // Without `--debug` the same image keeps the import-only table.
    let image = EncodedImage {
        debug_symbols: false,
        ..debug_image()
    };
    let parsed = parse_symtab(&encode_executable_bytes("dbg", &image).unwrap());
    assert_eq!(parsed.dysymtab, [0, 0, 0, 0, 0, 1]);
    assert_eq!(parsed.indirect, vec![0]);
    assert_eq!(parsed.symbols, vec![("_write".to_string(), 0x1, 0, 0)]);
    assert!(parsed.function_starts.is_empty());
}

#[test]
fn debug_symbols_on_import_less_image() {
    // No imports, initializers or constants: the chained-fixups path, which
    // writes no symtab at all without `--debug`.
    let image = EncodedImage {
        debug_symbols: true,
        ..exit_zero_image(Vec::new())
    };
    let parsed = parse_symtab(&encode_executable_bytes("dbg", &image).unwrap());
    let text = section_addr(&parsed, "__text");
    assert_eq!(parsed.symbols, vec![("_main".to_string(), 0x0e, 1, text)]);
    assert_eq!(parsed.dysymtab, [0, 1, 1, 0, 1, 0]);
    assert!(parsed.indirect.is_empty());
    assert_eq!(parsed.function_starts, vec![text - VM_BASE]);

    let plain =
        parse_symtab(&encode_executable_bytes("dbg", &exit_zero_image(Vec::new())).unwrap());
    assert!(plain.symbols.is_empty());
    assert_eq!(plain.dysymtab, [0; 6]);
}

#[test]
fn function_starts_lists_every_function() {
    let image = debug_image();
    let parsed = parse_symtab(&encode_executable_bytes("dbg", &image).unwrap());
    // `__text`'s file offset is its address minus the `__TEXT` base.
    let text_file_offset = section_addr(&parsed, "__text") - VM_BASE;
    let expected: Vec<u64> = ["_main", "_helper"]
        .iter()
        .map(|name| {
            let symbol = image.symbols.iter().find(|s| s.name == *name).unwrap();
            text_file_offset + symbol.offset as u64
        })
        .collect();
    assert_eq!(parsed.function_starts, expected);
}

/// The stub path with a shifted symtab still binds and runs: a `--debug` image
/// that calls two imports passes `codesign -v` and exits 0.
#[cfg(target_os = "macos")]
#[test]
fn debug_symbols_image_with_imports_runs() {
    let words: [u32; 6] = [
        0x9400_0000, // bl _nw_path_monitor_create
        0xB400_0060, // cbz x0, fail (+12)
        0xD280_0000, // movz x0, #0
        0x1400_0002, // b done (+8)
        0xD280_00E0, // fail: movz x0, #7
        0x9400_0000, // done: bl _exit
    ];
    let mut text = Vec::new();
    for word in words {
        put_u32(&mut text, word);
    }
    let external = |offset: usize, target: &str, library: &str| EncodedRelocation {
        offset,
        target: target.to_string(),
        kind: "branch26".to_string(),
        binding: "external".to_string(),
        library: Some(library.to_string()),
    };
    let image = EncodedImage {
        text,
        symbols: vec![text_symbol("_main", 0), text_symbol("_fail", 16)],
        relocations: vec![
            external(0, "_nw_path_monitor_create", "Network"),
            external(20, "_exit", "libSystem"),
        ],
        imports: vec![
            import("libSystem", "_exit"),
            import("Network", "_nw_path_monitor_create"),
        ],
        debug_symbols: true,
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "dbgrun", &image).expect("link");
    let verified = std::process::Command::new("/usr/bin/codesign")
        .arg("-v")
        .arg(&path)
        .status()
        .expect("run codesign");
    assert!(
        verified.success(),
        "codesign -v accepts the larger __LINKEDIT"
    );
    let status = std::process::Command::new(&path).status().expect("run");
    assert_eq!(status.code(), Some(0));
}

/// The 16 `LC_UUID` bytes of a Mach-O.
fn mach_o_uuid(bytes: &[u8]) -> [u8; 16] {
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut command = 32;
    for _ in 0..u32_at(16) {
        if u32_at(command) == 0x1b {
            return bytes[command + 8..command + 24].try_into().unwrap();
        }
        command += u32_at(command + 4) as usize;
    }
    panic!("LC_UUID must be present");
}

/// plan-166: a `--debug` build's UUID is derived from its contents, so two
/// different programs never share one (symbolicators cache names by UUID), and
/// rebuilding the same program keeps it. Without `--debug` it stays constant.
#[test]
fn debug_uuid_differs_between_images_and_is_stable() {
    let first = mach_o_uuid(&encode_executable_bytes("dbg", &debug_image()).unwrap());
    let again = mach_o_uuid(&encode_executable_bytes("dbg", &debug_image()).unwrap());
    let other_image = EncodedImage {
        debug_symbols: true,
        ..exit_zero_image(Vec::new())
    };
    let other = mach_o_uuid(&encode_executable_bytes("dbg", &other_image).unwrap());
    assert_eq!(first, again, "same program, same UUID");
    assert_ne!(first, other, "different programs, different UUIDs");
    assert_eq!(first[6] >> 4, 4, "RFC 4122 version 4");
    assert_eq!(first[8] >> 6, 0b10, "RFC 4122 variant");

    let constant = [0x4d, 0x46, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    for image in [
        EncodedImage {
            debug_symbols: false,
            ..debug_image()
        },
        exit_zero_image(Vec::new()),
    ] {
        assert_eq!(
            mach_o_uuid(&encode_executable_bytes("dbg", &image).unwrap()),
            constant
        );
    }
}
