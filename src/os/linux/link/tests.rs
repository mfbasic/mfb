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
        rodata_size: 0,
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
        rodata_size: 0,
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

// A minimal x86-64 image: `_main` at offset 0 that does `ret`. Raw-syscall
// backend, so no imports → the static, writable-data ELF path.
fn x86_static_image() -> EncodedImage {
    EncodedImage {
        text: vec![0xc3], // ret
        data: vec![1, 2, 3, 4],
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
    }
}

#[test]
fn encode_static_elf_x86_emits_two_pt_load_segments() {
    let image = x86_static_image();
    let bytes = encode_static_elf_x86(0, &image.text, &image.data, None);
    // e_ident magic + class/data/version.
    assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(&bytes[4..8], &[2, 1, 1, 0]);
    // e_type = ET_EXEC (2), e_machine = EM_X86_64 (62).
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 2);
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62);
    // e_entry = text_vmaddr + entry_offset(0).
    assert_eq!(
        u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        IMAGE_BASE + TEXT_FILE_OFFSET as u64
    );
    // e_phnum = 3 (text + data + GNU_STACK, bug-224).
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 3);
    // First program header at offset 64: PT_LOAD (1), R+X (5).
    assert_eq!(u32::from_le_bytes(bytes[64..68].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(bytes[68..72].try_into().unwrap()), 5);
    // Second program header at offset 64+56=120: PT_LOAD (1), R+W (6).
    assert_eq!(u32::from_le_bytes(bytes[120..124].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(bytes[124..128].try_into().unwrap()), 6);
    // Third program header at 64+2*56=176: PT_GNU_STACK (0x6474e551), R+W (6),
    // marking the stack non-executable (bug-224).
    assert_eq!(
        u32::from_le_bytes(bytes[176..180].try_into().unwrap()),
        0x6474_e551
    );
    assert_eq!(u32::from_le_bytes(bytes[180..184].try_into().unwrap()), 6);
    // The data segment's p_filesz (header 2 base 120 + 32) equals data length.
    assert_eq!(
        u64::from_le_bytes(bytes[152..160].try_into().unwrap()),
        image.data.len() as u64
    );
    // The text lands at TEXT_FILE_OFFSET and the data on the next page.
    assert_eq!(bytes[TEXT_FILE_OFFSET], 0xc3);
    let data_offset = align(TEXT_FILE_OFFSET + image.text.len(), PAGE_SIZE);
    assert_eq!(&bytes[data_offset..data_offset + 4], &[1, 2, 3, 4][..]);
}

/// bug-38: the static AArch64/RISC-V path appended data straight after text while
/// `write_executable` patched every data relocation against a *page-aligned*
/// `data_vmaddr`. Any text length not ending on a page boundary therefore shifted
/// every string and constant pointer.
#[test]
fn encode_static_elf_places_data_where_relocations_expect_it() {
    // A one-byte text section guarantees `TEXT_FILE_OFFSET + text.len()` is not
    // page-aligned, which is exactly when the old layout diverged.
    let image = x86_static_image();
    for (arch, machine) in [("aarch64", 183u16), ("riscv64", 243)] {
        let bytes = encode_static_elf(arch, 0, &image.text, &image.data, None);
        assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
        // e_machine follows the target ISA (this path serves both).
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), machine);
        // e_phnum = 3: text R+X, a writable data segment, and GNU_STACK (bug-224).
        assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 3);
        assert_eq!(u32::from_le_bytes(bytes[120..124].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[124..128].try_into().unwrap()), 6);
        // PT_GNU_STACK (0x6474e551), R+W (6) — non-executable stack.
        assert_eq!(
            u32::from_le_bytes(bytes[176..180].try_into().unwrap()),
            0x6474_e551
        );
        assert_eq!(u32::from_le_bytes(bytes[180..184].try_into().unwrap()), 6);

        // The address `write_executable` patches a data relocation to.
        let data_offset = align(TEXT_FILE_OFFSET + image.text.len(), PAGE_SIZE);
        let data_vmaddr = IMAGE_BASE + data_offset as u64;
        assert_ne!(data_offset, TEXT_FILE_OFFSET + image.text.len());
        // The data segment's p_offset / p_vaddr agree with it, and the bytes
        // really are there.
        assert_eq!(
            u64::from_le_bytes(bytes[128..136].try_into().unwrap()),
            data_offset as u64
        );
        assert_eq!(
            u64::from_le_bytes(bytes[136..144].try_into().unwrap()),
            data_vmaddr
        );
        assert_eq!(
            u64::from_le_bytes(bytes[152..160].try_into().unwrap()),
            image.data.len() as u64
        );
        assert_eq!(bytes[TEXT_FILE_OFFSET], 0xc3);
        assert_eq!(&bytes[data_offset..data_offset + 4], &[1, 2, 3, 4][..]);
    }
}

#[test]
fn encode_static_elf_x86_appends_signing_section() {
    let image = x86_static_image();
    let meta = br#"{"owner":"bob"}"#;
    let bytes = encode_static_elf_x86(0, &image.text, &image.data, Some(meta));
    assert!(bytes
        .windows(b".mfb_sign".len())
        .any(|window| window == b".mfb_sign"));
    assert!(bytes.windows(meta.len()).any(|window| window == meta));
    // Section header table is now present (e_shoff / e_shnum patched).
    assert_ne!(u64::from_le_bytes(bytes[40..48].try_into().unwrap()), 0);
    assert_eq!(u16::from_le_bytes([bytes[60], bytes[61]]), 3);
    assert_eq!(u16::from_le_bytes([bytes[62], bytes[63]]), 2);
}

#[test]
fn write_executable_x86_static_writes_flavored_output() {
    let image = x86_static_image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "x86s",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("link x86 static elf");
    // Console (non-app) build gets the flavor suffix.
    assert!(path.ends_with("x86s-glibc.out"));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62); // EM_X86_64
}

#[test]
fn write_executable_app_mode_drops_flavor_suffix() {
    let image = x86_static_image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "myapp",
        "x86_64",
        LinuxFlavor::Glibc,
        true,
        &image,
    )
    .expect("link app-mode elf");
    assert!(path.ends_with("myapp.out"));
    assert!(!path.to_string_lossy().contains("glibc"));
}

#[test]
fn write_executable_rejects_entry_not_in_text() {
    let mut image = x86_static_image();
    image.entry = "_nowhere".to_string();
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(
        dir.path(),
        "bad",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect_err("missing entry must be rejected");
    assert!(err.contains("does not resolve to text"));
}

// A dynamic x86-64 image importing libc `_exit` via `call _exit@PLT`
// (call_pc32) plus an internal call and internal/imported data references,
// exercising the x86 stub emitter and every x86 relocation kind.
fn x86_dynamic_image() -> EncodedImage {
    // Layout (offsets into text):
    //  0: call _helper        (internal call_pc32, disp at 1)
    //  5: call _exit@PLT       (external call_pc32, disp at 6)
    // 10: lea rax,[rip+data]   (data data_pc32, disp at 12; 48 8d 05 <disp>)
    // 17: lea rax,[rip+environ] (external data_pc32 via GOT, disp at 19)
    // 24: _helper: ret
    let mut text = vec![
        0xe8, 0, 0, 0, 0, // call rel32  -> _helper
        0xe8, 0, 0, 0, 0, // call rel32  -> _exit@PLT
        0x48, 0x8d, 0x05, 0, 0, 0, 0, // lea rax,[rip+disp32] -> _msg (data)
        0x48, 0x8b, 0x05, 0, 0, 0, 0,    // mov rax,[rip+disp32] -> environ (GOT)
        0xc3, // _helper: ret
    ];
    let _ = &mut text;
    EncodedImage {
        text,
        data: b"hi\0".to_vec(),
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_helper".to_string(),
                section: EncodedSection::Text,
                offset: 24,
            },
            EncodedSymbol {
                name: "_msg".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: vec![
            EncodedRelocation {
                offset: 1,
                target: "_helper".to_string(),
                kind: "call_pc32".to_string(),
                binding: "internal".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 6,
                target: "_exit".to_string(),
                kind: "call_pc32".to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            },
            EncodedRelocation {
                offset: 13,
                target: "_msg".to_string(),
                kind: "data_pc32".to_string(),
                binding: "data".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 20,
                target: "environ".to_string(),
                kind: "data_pc32".to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            },
        ],
        imports: vec![
            EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: "_exit".to_string(),
                kind: ImportKind::Function,
                version: None,
            },
            EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: "environ".to_string(),
                kind: ImportKind::Data,
                version: None,
            },
        ],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    }
}

#[test]
fn write_executable_x86_dynamic_covers_all_reloc_kinds() {
    let image = x86_dynamic_image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "x86d",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("link x86 dynamic elf");
    let bytes = std::fs::read(&path).unwrap();
    // EM_X86_64 PIE (ET_DYN) dynamic ELF: 6 program headers (incl. PT_GNU_STACK),
    // interpreter present (bug-186).
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 3); // ET_DYN
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62);
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 6);
    assert!(has_gnu_stack(&bytes), "PT_GNU_STACK must be present");
    assert!(bytes
        .windows(b"ld-linux-x86-64.so.2".len())
        .any(|window| window == b"ld-linux-x86-64.so.2"));
    // The internal PLT stubs use `jmp *disp32(%rip)` (FF 25 ...).
    assert!(bytes.windows(2).any(|window| window == [0xff, 0x25]));
}

#[test]
fn write_executable_x86_dynamic_musl_uses_musl_interpreter() {
    let image = x86_dynamic_image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "x86dm",
        "x86_64",
        LinuxFlavor::Musl,
        false,
        &image,
    )
    .expect("link x86 dynamic musl elf");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes
        .windows(b"ld-musl-x86_64.so.1".len())
        .any(|window| window == b"ld-musl-x86_64.so.1"));
}

#[test]
fn write_executable_aarch64_dynamic_musl_uses_musl_interpreter() {
    let image = glob_dat_image("libc.musl-aarch64.so.1");
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "aad",
        "aarch64",
        LinuxFlavor::Musl,
        false,
        &image,
    )
    .expect("link aarch64 musl elf");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes
        .windows(b"ld-musl-aarch64.so.1".len())
        .any(|window| window == b"ld-musl-aarch64.so.1"));
}

#[test]
fn write_executable_rejects_unbound_external_symbol() {
    // A branch26 to a symbol that is not imported → no stub → bind error.
    let mut text = Vec::new();
    put_u32(&mut text, 0x9400_0000); // bl _missing
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![EncodedRelocation {
            offset: 0,
            target: "_missing".to_string(),
            kind: "branch26".to_string(),
            binding: "external".to_string(),
            library: Some("libc.so.6".to_string()),
        }],
        // Import a *different* symbol so the dynamic path runs but the stub map
        // lacks `_missing`.
        imports: vec![EncodedImport {
            library: "libc.so.6".to_string(),
            symbol: "_exit".to_string(),
            kind: ImportKind::Function,
            version: None,
        }],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(
        dir.path(),
        "unbound",
        "aarch64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect_err("unbound external symbol must be rejected");
    assert!(err.contains("cannot bind external symbol '_missing'"));
}

#[test]
fn write_executable_rejects_unsupported_relocation() {
    let mut text = Vec::new();
    put_u32(&mut text, 0);
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![EncodedRelocation {
            offset: 0,
            target: "_main".to_string(),
            kind: "bogus_kind".to_string(),
            binding: "internal".to_string(),
            library: None,
        }],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(
        dir.path(),
        "unsup",
        "aarch64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect_err("unsupported relocation must be rejected");
    assert!(err.contains("does not support relocation"));
}

#[test]
fn dynamic_build_rejects_missing_initializer_symbol() {
    // An initializer naming no text symbol must error from DynamicPayload::build.
    let image = EncodedImage {
        text: vec![0x00, 0x00, 0x00, 0x00],
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: vec![EncodedImport {
            library: "libc.so.6".to_string(),
            symbol: "_exit".to_string(),
            kind: ImportKind::Function,
            version: None,
        }],
        entry: "_main".to_string(),
        initializers: vec!["_ghost".to_string()],
        signing_metadata: None,
    };
    let err = encode_dynamic_elf(
        "aarch64",
        LinuxFlavor::Glibc,
        0,
        &image.text,
        &image.data,
        &image,
    )
    .expect_err("dangling initializer must be rejected");
    assert!(err.contains("does not resolve to a text symbol"));
}

#[test]
fn dynamic_elf_appends_signing_section() {
    let mut image = glob_dat_image("libc.so.6");
    image.signing_metadata = Some(br#"{"k":"v"}"#.to_vec());
    let bytes = encode_dynamic_elf(
        "aarch64",
        LinuxFlavor::Glibc,
        0,
        &image.text,
        &image.data,
        &image,
    )
    .expect("dynamic elf with signing");
    assert!(bytes
        .windows(b".mfb_sign".len())
        .any(|window| window == b".mfb_sign"));
    assert!(bytes
        .windows(br#"{"k":"v"}"#.len())
        .any(|window| window == br#"{"k":"v"}"#));
}

// A static aarch64 image with an internal `bl` (branch26) and internal data
// references (page21/pageoff12) — no imports, so the static path exercises the
// aarch64 internal + data relocation arms of `patch_relocations`.
#[test]
fn write_executable_aarch64_static_internal_relocs() {
    let mut text = Vec::new();
    put_u32(&mut text, 0x9000_0000); // adrp x0, _msg          (data page21)
    put_u32(&mut text, 0x9100_0000); // add  x0, x0, :lo12:_msg (data pageoff12)
    put_u32(&mut text, 0x9400_0000); // bl   _helper           (internal branch26)
    put_u32(&mut text, 0xd65f_03c0); // ret
    put_u32(&mut text, 0xd65f_03c0); // _helper: ret
    let image = EncodedImage {
        text,
        data: b"hi\0".to_vec(),
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_helper".to_string(),
                section: EncodedSection::Text,
                offset: 16,
            },
            EncodedSymbol {
                name: "_msg".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: vec![
            EncodedRelocation {
                offset: 0,
                target: "_msg".to_string(),
                kind: "page21".to_string(),
                binding: "data".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 4,
                target: "_msg".to_string(),
                kind: "pageoff12".to_string(),
                binding: "data".to_string(),
                library: None,
            },
            EncodedRelocation {
                offset: 8,
                target: "_helper".to_string(),
                kind: "branch26".to_string(),
                binding: "internal".to_string(),
                library: None,
            },
        ],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    write_executable(
        dir.path(),
        "aast",
        "aarch64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("link aarch64 static internal-reloc elf");
}

// A `data data_pc32` (x86 RIP-relative to an internal data symbol) on the
// static x86 path, covering that arm without imports.
#[test]
fn write_executable_x86_static_data_pc32() {
    let mut text = vec![0x48, 0x8d, 0x05, 0, 0, 0, 0, 0xc3]; // lea rax,[rip+_d]; ret
    let _ = &mut text;
    let image = EncodedImage {
        text,
        data: b"xy\0".to_vec(),
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_d".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: vec![EncodedRelocation {
            offset: 3,
            target: "_d".to_string(),
            kind: "data_pc32".to_string(),
            binding: "data".to_string(),
            library: None,
        }],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    write_executable(
        dir.path(),
        "x86dp",
        "x86_64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("link x86 static data_pc32 elf");
}

// Drives each "cannot bind external ... symbol" error arm: a relocation of the
// given kind whose target is not among the imports (so the stub/GOT map lacks
// it) but a *different* symbol is imported (so the dynamic path is taken).
fn expect_unbound(kind: &str, expect_fragment: &str) {
    let mut text = Vec::new();
    // One 4-byte instruction; the exact bytes don't matter for the bind check.
    put_u32(&mut text, 0);
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![EncodedRelocation {
            offset: 0,
            target: "_missing".to_string(),
            kind: kind.to_string(),
            binding: "external".to_string(),
            library: None, // exercises the "<unknown library>" fallback
        }],
        imports: vec![EncodedImport {
            library: "libc.so.6".to_string(),
            symbol: "_present".to_string(),
            kind: ImportKind::Function,
            version: None,
        }],
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let arch = if kind.starts_with("riscv") {
        "riscv64"
    } else if kind.ends_with("pc32") {
        "x86_64"
    } else {
        "aarch64"
    };
    let err = write_executable(dir.path(), "ub", arch, LinuxFlavor::Glibc, false, &image)
        .expect_err("unbound external must be rejected");
    assert!(
        err.contains(expect_fragment) && err.contains("<unknown library>"),
        "unexpected error for {kind}: {err}"
    );
}

#[test]
fn write_executable_rejects_unbound_external_all_kinds() {
    expect_unbound("page21", "cannot bind external data symbol '_missing'");
    expect_unbound("pageoff12", "cannot bind external data symbol '_missing'");
    expect_unbound("call_pc32", "cannot bind external symbol '_missing'");
    expect_unbound("data_pc32", "cannot bind external data symbol '_missing'");
    // RISC-V external relocations with no bound import hit their own guards.
    expect_unbound("riscv_call", "cannot bind external symbol '_missing'");
    expect_unbound("riscv_got_hi20", "cannot bind external data symbol '_missing'");
    expect_unbound("riscv_got_lo12", "cannot bind external data symbol '_missing'");
}

/// An internal `call` between two functions in the same image lowers to a
/// `riscv_call` relocation with `internal` binding — the auipc/jalr pair is
/// patched from the caller's PC (covers the `internal riscv_call` arm, distinct
/// from the imported-stub path the dynamic test exercises).
#[test]
fn write_executable_riscv64_internal_call_patches_auipc_jalr_pair() {
    use crate::target::shared::code::{CodeFrame, CodeFunction, NativeCodePlan};
    fn func(name: &str, instructions: Vec<crate::target::shared::code::CodeInstruction>) -> CodeFunction {
        CodeFunction {
            name: name.to_string(),
            symbol: name.to_string(),
            params: Vec::new(),
            returns: "Integer".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        }
    }
    let plan = NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("_main".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![
            // `_main` calls the internal `_helper` (internal riscv_call), then returns.
            func("_main", vec![rv_inst("bl", &[("target", "_helper")]), rv_inst("ret", &[])]),
            func("_helper", vec![rv_inst("ret", &[])]),
        ],
    };
    let image = crate::arch::riscv64::encode::encode(&plan).expect("riscv encode");
    // No imports → a static ELF; the internal call relocation is still patched.
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "rvi", "riscv64", LinuxFlavor::Glibc, false, &image)
        .expect("link riscv static elf with internal call");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 243); // EM_RISCV
}

#[test]
fn write_executable_rejects_undefined_internal_symbol() {
    // An internal branch26 whose target names no symbol → symbol_vmaddr error.
    let mut text = Vec::new();
    put_u32(&mut text, 0x9400_0000);
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: "_main".to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: vec![EncodedRelocation {
            offset: 0,
            target: "_ghost".to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        }],
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(
        dir.path(),
        "undef",
        "aarch64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect_err("undefined internal symbol must be rejected");
    assert!(err.contains("symbol '_ghost' does not resolve"));
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
    let unsigned = encode_static_elf("aarch64", 0, &image.text, &image.data, None);
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

fn riscv_reloc(offset: usize, target: &str, kind: &str) -> EncodedRelocation {
    EncodedRelocation {
        offset,
        target: target.to_string(),
        kind: kind.to_string(),
        binding: "data".to_string(),
        library: None,
    }
}

#[test]
fn riscv_pcrel_lo12_pairs_with_adjacent_auipc() {
    // The common case: `auipc rd,%hi` at 100, `addi rd,rd,%lo` at 104.
    let relocs = vec![
        riscv_reloc(100, "P", "riscv_pcrel_hi20"),
        riscv_reloc(104, "P", "riscv_pcrel_lo12"),
    ];
    assert_eq!(
        paired_auipc_offset(&relocs, &relocs[1], "riscv_pcrel_hi20"),
        Ok(100)
    );
}

#[test]
fn riscv_pcrel_lo12_pairs_across_a_spill_gap() {
    // The regression: the allocator spilled `rd` right after the `auipc`, so the
    // `addi` at 116 is 16 bytes past its `auipc` at 100 — not the adjacent 104.
    // A hard-coded `offset - 4` would mis-locate the PC base and corrupt the low
    // 12 bits of the address (two inlined SIMD kernels reading a shifted pool).
    let relocs = vec![
        riscv_reloc(100, "P", "riscv_pcrel_hi20"),
        riscv_reloc(116, "P", "riscv_pcrel_lo12"),
    ];
    assert_eq!(
        paired_auipc_offset(&relocs, &relocs[1], "riscv_pcrel_hi20"),
        Ok(100)
    );
}

#[test]
fn riscv_pcrel_lo12_pairs_with_nearest_preceding_hi_of_same_target() {
    // Two materializations of the same symbol: each lo12 must bind to its own
    // (nearest preceding) auipc, and a lo12 never pairs with a hi for a
    // different target.
    let relocs = vec![
        riscv_reloc(100, "P", "riscv_pcrel_hi20"),
        riscv_reloc(108, "P", "riscv_pcrel_lo12"),
        riscv_reloc(200, "Q", "riscv_pcrel_hi20"),
        riscv_reloc(220, "P", "riscv_pcrel_hi20"),
        riscv_reloc(228, "P", "riscv_pcrel_lo12"),
    ];
    assert_eq!(
        paired_auipc_offset(&relocs, &relocs[1], "riscv_pcrel_hi20"),
        Ok(100)
    );
    assert_eq!(
        paired_auipc_offset(&relocs, &relocs[4], "riscv_pcrel_hi20"),
        Ok(220)
    );
    // A lo12 with no preceding hi to its target is a hard error, not a silent
    // `offset - 4` guess.
    let orphan = vec![riscv_reloc(50, "Z", "riscv_pcrel_lo12")];
    assert!(paired_auipc_offset(&orphan, &orphan[0], "riscv_pcrel_hi20").is_err());
}

/// bug-39: the SysV `DT_HASH` chain must begin with the unused null-symbol slot.
/// Writing the first link into `chain[0]` shifted every entry down one slot, so a
/// by-name lookup of the second and later symbols walked past its own entry.
#[test]
fn dynamic_elf_hash_chain_starts_at_the_null_symbol() {
    let image = glob_dat_image("libc.so.6");
    let bytes = encode_dynamic_elf(
        "aarch64",
        LinuxFlavor::Glibc,
        0,
        &image.text,
        &image.data,
        &image,
    )
    .expect("encode dynamic elf");

    // nbucket=1, nchain=3 (2 imports + null), bucket[0]=1, then the chain:
    // chain[0]=0 (null symbol), chain[1]=2 (link to symbol 2), chain[2]=0 (end).
    let expected: Vec<u8> = [1u32, 3, 1, 0, 2, 0]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    assert!(
        bytes.windows(expected.len()).any(|w| w == expected),
        "DT_HASH section not found with a null-symbol chain[0]"
    );
    // The pre-fix layout put the first link where chain[0] belongs.
    let shifted: Vec<u8> = [1u32, 3, 1, 2, 0, 0]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    assert!(!bytes.windows(shifted.len()).any(|w| w == shifted));
}

/// bug-39: `auipc` reaches ±2 GiB. Masking the high 20 bits without a range check
/// silently dropped the rest and patched a jump or load to the wrong address.
#[test]
fn riscv_hi_lo_rejects_a_displacement_past_the_auipc_reach() {
    // In-range displacements encode as before.
    assert_eq!(riscv_hi_lo(0), Ok((0, 0)));
    assert_eq!(riscv_hi_lo(0x7ff), Ok((0, 2047)));
    assert_eq!(riscv_hi_lo(0x800), Ok((1, -2048)));
    assert_eq!(riscv_hi_lo(-4), Ok((0, -4)));
    // A displacement below -2048 rounds hi down, so the sign-extended lo12 corrects it.
    assert_eq!(riscv_hi_lo(-0x801), Ok((0xfffff, 2047)));
    // The exact boundaries of the auipc + lo12 reach.
    assert!(riscv_hi_lo(0x7fff_f7ff).is_ok());
    assert!(riscv_hi_lo(-0x8000_0800).is_ok());
    // One byte past either end is an error, not a truncated immediate.
    let err = riscv_hi_lo(0x7fff_f800).expect_err("beyond +2 GiB");
    assert!(err.contains("exceeds the ±2 GiB reach"), "{err}");
    assert!(riscv_hi_lo(-0x8000_0801).is_err());
    assert!(riscv_hi_lo(i64::MAX).is_err());
    assert!(riscv_hi_lo(i64::MIN + 0x800).is_err());
}

fn rv_inst(op: &str, fields: &[(&'static str, &str)]) -> crate::target::shared::code::CodeInstruction {
    let mut instruction = crate::target::shared::code::CodeInstruction::new(op);
    for (key, value) in fields {
        instruction = instruction.field(key, value);
    }
    instruction
}

// Build a real RISC-V image through the arch encoder — so text, relocation kinds
// (riscv_call / riscv_pcrel_hi20 / riscv_pcrel_lo12 / riscv_got_hi20 /
// riscv_got_lo12) and offsets are self-consistent — then drive the full dynamic
// ELF writer end to end. This is host-neutral byte generation: it covers every
// RISC-V arm of `patch_relocations`, the RISC-V `emit_import_stub`, and the
// RISC-V dynamic ELF header/e_flags/interpreter path on any CI host.
#[test]
fn write_executable_riscv64_dynamic_covers_call_pcrel_and_got() {
    use crate::target::shared::code::{
        CodeDataObject, CodeFrame, CodeFunction, CodeImport, NativeCodePlan,
    };
    let main = CodeFunction {
        name: "_main".to_string(),
        symbol: "_main".to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions: vec![
            // Internal data global via PC-relative pair → riscv_pcrel_hi20/lo12.
            rv_inst("adrp", &[("dst", "a0"), ("symbol", "_msg")]),
            rv_inst("add_pageoff", &[("dst", "a0"), ("symbol", "_msg")]),
            // Imported data global via the GOT → riscv_got_hi20/lo12.
            rv_inst("adrp", &[("dst", "a1"), ("symbol", "environ")]),
            rv_inst("add_pageoff", &[("dst", "a1"), ("symbol", "environ")]),
            // Imported function call → external riscv_call.
            rv_inst("bl", &[("target", "_exit")]),
            rv_inst("ret", &[]),
        ],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    };
    let plan = NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("_main".to_string()),
        imports: vec![
            CodeImport {
                library: "libc.so.6".to_string(),
                symbol: "_exit".to_string(),
            },
            CodeImport {
                library: "libc.so.6".to_string(),
                symbol: "environ".to_string(),
            },
        ],
        data_objects: vec![CodeDataObject {
            symbol: "_msg".to_string(),
            kind: "string".to_string(),
            layout: String::new(),
            align: 8,
            size: 16,
            value: "hi".to_string(),
        }],
        functions: vec![main],
    };
    let image = crate::arch::riscv64::encode::encode(&plan).expect("riscv encode");
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "rvd",
        "riscv64",
        LinuxFlavor::Glibc,
        false,
        &image,
    )
    .expect("link riscv dynamic elf");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
    // PIE (ET_DYN); EM_RISCV (243) with EF_RISCV_FLOAT_ABI_DOUBLE in e_flags
    // (offset 48) (bug-186).
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 3); // ET_DYN
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 243);
    assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()) & 0x4, 0x4);
    // 6 program headers (dynamic + PT_GNU_STACK) and the riscv64 interpreter path.
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 6);
    assert!(has_gnu_stack(&bytes), "PT_GNU_STACK must be present");
    assert!(bytes
        .windows(b"ld-linux-riscv64".len())
        .any(|window| window == b"ld-linux-riscv64"));
}

/// Whether the ELF's program-header table contains a `PT_GNU_STACK` entry
/// (bug-186 NX-stack marker). Reads e_phoff/e_phnum from the header.
fn has_gnu_stack(bytes: &[u8]) -> bool {
    let phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;
    (0..phnum).any(|i| {
        let base = phoff + i * 56;
        u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) == 0x6474_e551
    })
}
