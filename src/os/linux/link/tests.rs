use super::*;
use crate::arch::aarch64::encode::{EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind};

fn versioned_exit_image() -> EncodedImage {
    // _main: movz w0, #0 ; bl _exit  (exit(0) through a versioned reference).
    let mut text = Vec::new();
    put_u32(&mut text, 0xd280_0000);
    put_u32(&mut text, 0x9400_0000);
    EncodedImage {
        text,
        data: Vec::new(),
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![EncodedRelocation {
            offset: 4,
            target: "_exit".to_string(),
            kind: "branch26".to_string(),
            binding: "external".to_string(),
            library: Some("libc.so.6".to_string()),
        }],
        imports: vec![EncodedImport {
            library: "libc.so.6".to_string(),
            symbol: "_exit".to_string(),
            kind: ImportKind::Function,
            version: Some("GLIBC_2.17".to_string()),
        }],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    }
}

// A program whose load-time initializer sets a data global that `main` reads
// (plan-linker.md §6.4): `main` exits 0 only if the initializer ran first.
fn init_array_image() -> EncodedImage {
    let mut text = Vec::new();
    // _init0 @0: _flag = 42.
    put_u32(&mut text, 0x9000_0000); // adrp x0, _flag         (page21)
    put_u32(&mut text, 0x9100_0000); // add  x0, x0, :lo12:_flag (pageoff12)
    put_u32(&mut text, 0xd280_0541); // movz x1, #42
    put_u32(&mut text, 0xf900_0001); // str  x1, [x0]
    put_u32(&mut text, 0xd65f_03c0); // ret
                                     // _main @20: exit(_flag == 42 ? 0 : 1).
    put_u32(&mut text, 0x9000_0000); // adrp x0, _flag         (page21)
    put_u32(&mut text, 0x9100_0000); // add  x0, x0, :lo12:_flag (pageoff12)
    put_u32(&mut text, 0xf940_0000); // ldr  x0, [x0]
    put_u32(&mut text, 0xf100_a81f); // cmp  x0, #42
    put_u32(&mut text, 0x5280_0000); // movz w0, #0
    put_u32(&mut text, 0x5400_0040); // b.eq +8
    put_u32(&mut text, 0x5280_0020); // movz w0, #1
    put_u32(&mut text, 0x9400_0000); // bl   _exit             (branch26)
    let data_reloc = |offset: usize, kind: &str| EncodedRelocation {
        offset,
        target: "_flag".to_string(),
        kind: kind.to_string(),
        binding: "data".to_string(),
        library: None,
    };
    EncodedImage {
        text,
        data: vec![0; 8],
        symbols: vec![
            EncodedSymbol {
                name: "_init0".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 20,
            },
            EncodedSymbol {
                name: "_flag".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: vec![
            data_reloc(0, "page21"),
            data_reloc(4, "pageoff12"),
            data_reloc(20, "page21"),
            data_reloc(24, "pageoff12"),
            EncodedRelocation {
                offset: 48,
                target: "_exit".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            },
        ],
        imports: vec![EncodedImport {
            library: "libc.so.6".to_string(),
            symbol: "_exit".to_string(),
            kind: ImportKind::Function,
            version: None,
        }],
        entry: "_main".to_string(),
        initializers: vec!["_init0".to_string()],
        signing_metadata: None,
    }
}

// Reads the real glibc data global `environ` (a `char**`) through the GOT via
// a GLOB_DAT relocation (plan-linker.md §6.1) and exits 0 iff it is non-null,
// proving the import resolved to libc's data symbol.
fn glob_dat_image(libc: &str) -> EncodedImage {
    let mut text = Vec::new();
    put_u32(&mut text, 0x9000_0000); // adrp x0, environ        (external page21)
    put_u32(&mut text, 0x9100_0000); // add  x0, x0, :got_lo12  (external pageoff12)
    put_u32(&mut text, 0xf940_0000); // ldr  x0, [x0]   ; x0 = &environ
    put_u32(&mut text, 0xf940_0000); // ldr  x0, [x0]   ; x0 = environ (envp)
    put_u32(&mut text, 0xf100_001f); // cmp  x0, #0
    put_u32(&mut text, 0x5280_0000); // movz w0, #0     ; success default
    put_u32(&mut text, 0x5400_0041); // b.ne +8         ; non-null -> keep 0
    put_u32(&mut text, 0x5280_0020); // movz w0, #1
    put_u32(&mut text, 0x9400_0000); // bl   _exit              (external branch26)
    let ext = |offset: usize, target: &str, kind: &str| EncodedRelocation {
        offset,
        target: target.to_string(),
        kind: kind.to_string(),
        binding: "external".to_string(),
        library: Some(libc.to_string()),
    };
    EncodedImage {
        text,
        data: Vec::new(),
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![
            ext(0, "environ", "page21"),
            ext(4, "environ", "pageoff12"),
            ext(32, "_exit", "branch26"),
        ],
        imports: vec![
            EncodedImport {
                library: libc.to_string(),
                symbol: "environ".to_string(),
                kind: ImportKind::Data,
                version: None,
            },
            EncodedImport {
                library: libc.to_string(),
                symbol: "_exit".to_string(),
                kind: ImportKind::Function,
                version: None,
            },
        ],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    }
}

fn text_symbol(name: &str, offset: usize) -> EncodedSymbol {
    EncodedSymbol {
        name: name.to_string(),
        section: EncodedSection::Text,
        offset,
    }
}

fn ret_image() -> EncodedImage {
    EncodedImage {
        // x86 `ret` (0xc3) padded so text is a few bytes.
        text: vec![0xc3, 0x90, 0x90, 0x90],
        data: vec![1, 2, 3, 4],
        symbols: vec![text_symbol("_main", 0)],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    }
}

// The x86-64 static path (no imports → raw-syscall ELF): two PT_LOAD segments
// (text R+X, data R+W). Asserts the header machine/type and both program
// headers, driving `encode_static_elf_x86` and the `arch == "x86_64"` dispatch.
#[test]
fn writes_static_x86_elf_with_writable_data_segment() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "prog",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &ret_image(),
    )
    .expect("static x86 elf");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..4], b"\x7fELF");
    // e_machine = EM_X86_64 (62).
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62);
    // e_phnum = 2 (text + data).
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 2);
    // Second program header (data) has p_flags = R+W (6) at phoff 64 + 56.
    let ph2 = 64 + 56;
    assert_eq!(
        u32::from_le_bytes(bytes[ph2 + 4..ph2 + 8].try_into().unwrap()),
        6
    );
}

#[test]
fn app_mode_writes_unflavored_output_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "app",
        "x86_64",
        LinuxFlavor::Glibc,
        true,
        &ret_image(),
    )
    .expect("app-mode elf");
    // App mode drops the flavor suffix.
    assert_eq!(path, dir.path().join("app.out"));
}

#[test]
fn static_x86_elf_carries_signing_section() {
    let mut image = ret_image();
    image.signing_metadata = Some(br#"{"owner":"bob"}"#.to_vec());
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "signed",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("signed static x86");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.windows(b".mfb_sign".len()).any(|w| w == b".mfb_sign"));
    assert!(bytes
        .windows(br#"{"owner":"bob"}"#.len())
        .any(|w| w == br#"{"owner":"bob"}"#));
}

// The x86-64 dynamic path: a `call sym@PLT` (external call_pc32), a `lea
// rip-rel` (internal call_pc32) and an imported data global via GOTPCREL
// (external data_pc32). Drives `encode_dynamic_elf(arch="x86_64")`, the x86 PLT
// stub, and the x86 relocation arms.
#[test]
fn writes_dynamic_x86_elf_with_plt_and_gotpcrel() {
    let mut text = Vec::new();
    // call helper (E8 rel32) at 0; disp32 field at offset 1.
    text.extend_from_slice(&[0xe8, 0, 0, 0, 0]);
    // lea rax,[rip+environ_got] (48 8D 05 disp32); disp32 field at offset 8.
    text.extend_from_slice(&[0x48, 0x8d, 0x05, 0, 0, 0, 0]);
    // call _exit@PLT (E8 rel32) at 12; disp32 field at offset 13.
    text.extend_from_slice(&[0xe8, 0, 0, 0, 0]);
    // helper: ret.
    text.push(0xc3);
    let image = EncodedImage {
        text,
        data: Vec::new(),
        symbols: vec![text_symbol("_main", 0), text_symbol("helper", 17)],
        relocations: vec![
            EncodedRelocation {
                offset: 1,
                target: "helper".to_string(),
                kind: "call_pc32".to_string(),
                binding: "internal".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 8,
                target: "environ".to_string(),
                kind: "data_pc32".to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            },
            EncodedRelocation {
                offset: 13,
                target: "_exit".to_string(),
                kind: "call_pc32".to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            },
        ],
        imports: vec![
            EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: "environ".to_string(),
                kind: ImportKind::Data,
                version: None,
            },
            EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: "_exit".to_string(),
                kind: ImportKind::Function,
                version: None,
            },
        ],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = std::path::PathBuf::from("tmp/x86dyn");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = write_executable(&dir, "x86dyn", "x86_64", LinuxFlavor::Glibc, false, &image)
        .expect("dynamic x86 elf");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62); // EM_X86_64
    assert!(bytes.windows(b"environ".len()).any(|w| w == b"environ"));
    // The x86 interpreter path reached ld-linux-x86-64.
    assert!(bytes
        .windows(b"ld-linux-x86-64.so.2".len())
        .any(|w| w == b"ld-linux-x86-64.so.2"));
}

#[test]
fn writes_dynamic_x86_musl_elf() {
    let mut text = Vec::new();
    text.extend_from_slice(&[0xe8, 0, 0, 0, 0]); // call _exit@PLT
    let image = EncodedImage {
        text,
        data: Vec::new(),
        symbols: vec![text_symbol("_main", 0)],
        relocations: vec![EncodedRelocation {
            offset: 1,
            target: "_exit".to_string(),
            kind: "call_pc32".to_string(),
            binding: "external".to_string(),
            library: Some("libc.musl-x86_64.so.1".to_string()),
        }],
        imports: vec![EncodedImport {
            library: "libc.musl-x86_64.so.1".to_string(),
            symbol: "_exit".to_string(),
            kind: ImportKind::Function,
            version: None,
        }],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = std::path::PathBuf::from("tmp/x86musldyn");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = write_executable(&dir, "x86musl", "x86_64", LinuxFlavor::Musl, false, &image)
        .expect("dynamic x86 musl elf");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes
        .windows(b"ld-musl-x86_64.so.1".len())
        .any(|w| w == b"ld-musl-x86_64.so.1"));
}

#[test]
fn patch_relocations_supports_data_pc32_internal() {
    // `data` binding data_pc32: a RIP-relative address of an internal data symbol.
    let mut text = vec![0x48, 0x8d, 0x05, 0, 0, 0, 0];
    let image = EncodedImage {
        text: text.clone(),
        data: vec![0; 8],
        symbols: vec![
            text_symbol("_main", 0),
            EncodedSymbol {
                name: "_g".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: vec![EncodedRelocation {
            offset: 3,
            target: "_g".to_string(),
            kind: "data_pc32".to_string(),
            binding: "data".to_string(),
            library: None,
        }],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    patch_relocations(
        &mut text,
        &image,
        IMAGE_BASE + TEXT_FILE_OFFSET as u64,
        IMAGE_BASE + 0x2000,
        &ImportLocations::default(),
    )
    .expect("data_pc32 internal");
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
        IMAGE_BASE,
        IMAGE_BASE,
        &ImportLocations::default(),
    )
    .expect_err("unsupported reloc");
    assert!(err.contains("does not support relocation"), "{err}");
}

#[test]
fn patch_relocations_rejects_unbound_externals_each_arch() {
    let cases = [
        ("branch26", "aarch64"),
        ("page21", "aarch64"),
        ("pageoff12", "aarch64"),
        ("call_pc32", "x86_64"),
        ("data_pc32", "x86_64"),
    ];
    for (kind, arch) in cases {
        let image = EncodedImage {
            text: vec![0; 8],
            data: Vec::new(),
            symbols: vec![text_symbol("_main", 0)],
            relocations: vec![EncodedRelocation {
                offset: 0,
                target: "_unbound".to_string(),
                kind: kind.to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            }],
            imports: Vec::new(),
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
        };
        let mut text = image.text.clone();
        let err = patch_relocations(
            &mut text,
            &image,
            IMAGE_BASE,
            IMAGE_BASE,
            &ImportLocations::default(),
        )
        .expect_err("unbound external");
        assert!(err.contains("cannot bind external"), "{arch}/{kind}: {err}");
    }
}

#[test]
fn symbol_vmaddr_resolves_sections_and_rejects_unknown() {
    let image = EncodedImage {
        text: Vec::new(),
        data: Vec::new(),
        symbols: vec![
            text_symbol("_t", 4),
            EncodedSymbol {
                name: "_d".to_string(),
                section: EncodedSection::Data,
                offset: 8,
            },
        ],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    assert_eq!(symbol_vmaddr(&image, "_t", 0x1000, 0x2000).unwrap(), 0x1004);
    assert_eq!(symbol_vmaddr(&image, "_d", 0x1000, 0x2000).unwrap(), 0x2008);
    assert!(symbol_vmaddr(&image, "_missing", 0x1000, 0x2000)
        .expect_err("unknown")
        .contains("does not resolve"));
}

#[test]
fn rejects_entry_symbol_not_in_text() {
    let image = EncodedImage {
        text: vec![0xc3],
        data: vec![0; 4],
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Data,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(
        dir.path(),
        "bad",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect_err("entry not text");
    assert!(err.contains("does not resolve to text"), "{err}");
}

#[test]
fn elf_hash_matches_sysv_reference() {
    // Reference SysV hash values (used for Vernaux.vna_hash).
    assert_eq!(elf_hash(b""), 0);
    assert_eq!(elf_hash(b"GLIBC_2.17"), 0x0696_9197);
}

#[test]
fn branch_imm26_encodes_relative_word_offset() {
    // Forward branch of 8 bytes → 2 words.
    assert_eq!(branch_imm26(0, 8), 2);
    // Backward branch of 4 bytes → -1 word masked to 26 bits.
    assert_eq!(branch_imm26(8, 4), 0x03ff_ffff);
}

#[test]
fn align_rounds_up_power_of_two() {
    assert_eq!(align(0, 0x1000), 0);
    assert_eq!(align(1, 0x1000), 0x1000);
    assert_eq!(align(0x1000, 0x1000), 0x1000);
    assert_eq!(align(0x1001, 0x1000), 0x2000);
}

#[test]
fn writes_glob_dat_glibc_elf() {
    let image = glob_dat_image("libc.so.6");
    let dir = std::path::PathBuf::from("tmp/globlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(&dir, "glob", "aarch64", LinuxFlavor::Glibc, false, &image)
        .expect("link glob_dat elf");
}

#[test]
fn writes_glob_dat_musl_elf() {
    let image = glob_dat_image("libc.musl-aarch64.so.1");
    let dir = std::path::PathBuf::from("tmp/globlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(
        &dir,
        "globmusl",
        "aarch64",
        LinuxFlavor::Musl,
        false,
        &image,
    )
    .expect("link musl glob_dat");
}

#[test]
fn writes_mfb_sign_section_to_static_elf() {
    let mut image = EncodedImage {
        text: vec![0xd6, 0x5f, 0x03, 0xc0],
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
    let path = write_executable(
        dir.path(),
        "signed",
        "aarch64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("link signed elf");
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes
        .windows(b".mfb_sign".len())
        .any(|window| window == b".mfb_sign"));
    assert!(bytes
        .windows(br#"{"owner":"alice"}"#.len())
        .any(|window| window == br#"{"owner":"alice"}"#));
    assert_eq!(u16::from_le_bytes([bytes[60], bytes[61]]), 3);
    assert_eq!(u16::from_le_bytes([bytes[62], bytes[63]]), 2);
    image.signing_metadata = None;
    let unsigned = encode_static_elf(0, &image.text, &image.data, None);
    assert_eq!(u64::from_le_bytes(unsigned[40..48].try_into().unwrap()), 0);
}

// Confirms DT_INIT_ARRAY / DT_INIT_ARRAYSZ are emitted for the listed
// initializers (verified with `readelf -d`). Note: glibc runs the *main
// executable's* init_array from the CRT (`__libc_start_main`), not from
// `ld.so`, so a custom-entry binary like this one does not invoke it at load;
// the array is emitted for CRT/shared-object scenarios and for parity with
// the macOS mod-init path (plan-linker.md §6.4).
#[test]
fn writes_init_array_glibc_elf() {
    let image = init_array_image();
    let dir = std::path::PathBuf::from("tmp/initlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(&dir, "init", "aarch64", LinuxFlavor::Glibc, false, &image)
        .expect("link init-array elf");
}

// Emits a dynamic glibc ELF whose single import requires `_exit@GLIBC_2.17`,
// exercising the verneed/versym path (plan-linker.md §6.2). The byte check
// confirms the version string reaches `.dynstr`; the file is left under
// `tmp/verlx` so it can be executed against a real glibc `ld.so` (which
// rejects a missing/mismatched version at load) to prove the structure.
#[test]
fn writes_versioned_glibc_elf() {
    let image = versioned_exit_image();
    let dir = std::path::PathBuf::from("tmp/verlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = write_executable(&dir, "ver", "aarch64", LinuxFlavor::Glibc, false, &image)
        .expect("link versioned elf");
    let bytes = std::fs::read(&path).expect("read elf");
    assert!(
        bytes
            .windows("GLIBC_2.17".len())
            .any(|window| window == b"GLIBC_2.17"),
        ".dynstr should contain the required version GLIBC_2.17"
    );
}
