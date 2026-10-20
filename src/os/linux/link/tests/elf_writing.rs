//! plan-176-J: the Linux ELF linker's elf_writing tests. The shared image builders live in\n//! `link/tests/mod.rs`.

use super::*;

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
        rpaths: Vec::new(),
        debug_symbols: false,
    }
}

// ===== Static ELF layout + sizing =====

#[test]
fn encode_static_elf_x86_emits_two_pt_load_segments() {
    let image = x86_static_image();
    let bytes = encode_static_elf_x86(0, &image.text, &image.data, None, None);
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
    // e_phnum = 4 (text + data + GNU_STACK (bug-224) + PT_NOTE (plan-43)).
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 4);
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
        let bytes = encode_static_elf(arch, 0, &image.text, &image.data, None, None);
        assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
        // e_machine follows the target ISA (this path serves both).
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), machine);
        // e_phnum = 4: text R+X, a writable data segment, GNU_STACK (bug-224),
        // and the PT_NOTE provenance marker (plan-43).
        assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 4);
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
    let bytes = encode_static_elf_x86(0, &image.text, &image.data, Some(meta), None);
    assert!(bytes
        .windows(b".mfbsign".len())
        .any(|window| window == b".mfbsign"));
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
    let path = write_executable(dir.path(), "x86s", "x86_64", LinuxFlavor::Glibc, &image)
        .expect("link x86 static elf");
    // Console (non-app) build gets the flavor suffix.
    assert!(path.ends_with("x86s-glibc.out"));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62); // EM_X86_64
}

/// plan-51-A: app mode no longer produces a bare `<name>.out`; it produces an
/// AppDir whose executable is at `usr/bin/<name>`, encoded from the same bytes
/// the console path would write.
#[test]
fn write_appdir_puts_the_same_elf_under_usr_bin() {
    let image = x86_static_image();
    let dir = tempfile::tempdir().unwrap();
    let appdir = write_appdir(
        dir.path(),
        "myapp",
        "x86_64",
        LinuxFlavor::Glibc,
        &image,
        None,
        "0.1.0",
    )
    .expect("link app-mode AppDir");
    assert!(appdir.ends_with("myapp-glibc.AppDir"));
    assert!(
        !appdir.join("..").join("myapp.out").exists(),
        "app mode emits no bare <name>.out"
    );

    // The AppDir's ELF is byte-identical to the console `<name>-glibc.out` from
    // the same image: this build vendors nothing, so neither carries a
    // DT_RUNPATH and the only difference would be one (plan-51-A §4.4).
    let console = tempfile::tempdir().unwrap();
    let out = write_executable(
        console.path(),
        "myapp",
        "x86_64",
        LinuxFlavor::Glibc,
        &image,
    )
    .expect("link console elf");
    assert_eq!(
        std::fs::read(appdir.join("usr/bin/myapp")).unwrap(),
        std::fs::read(&out).unwrap(),
        "one image, one encoder, identical bytes"
    );
}

#[test]
fn write_executable_rejects_entry_not_in_text() {
    let mut image = x86_static_image();
    image.entry = "_nowhere".to_string();
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(dir.path(), "bad", "x86_64", LinuxFlavor::Glibc, &image)
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
        ..none()
    }
}

#[test]
fn write_executable_x86_dynamic_covers_all_reloc_kinds() {
    let image = x86_dynamic_image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "x86d", "x86_64", LinuxFlavor::Glibc, &image)
        .expect("link x86 dynamic elf");
    let bytes = std::fs::read(&path).unwrap();
    // EM_X86_64 PIE (ET_DYN) dynamic ELF: 8 program headers (incl. PT_GNU_STACK,
    // the plan-43 PT_NOTE, and the bug-263 PT_GNU_RELRO), interpreter present
    // (bug-186).
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 3); // ET_DYN
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62);
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 8);
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
    let path = write_executable(dir.path(), "x86dm", "x86_64", LinuxFlavor::Musl, &image)
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
    let path = write_executable(dir.path(), "aad", "aarch64", LinuxFlavor::Musl, &image)
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
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(dir.path(), "unbound", "aarch64", LinuxFlavor::Glibc, &image)
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
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(dir.path(), "unsup", "aarch64", LinuxFlavor::Glibc, &image)
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
        rpaths: Vec::new(),
        debug_symbols: false,
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
    image.signing_metadata = Some(crate::arch::image::ExecutableSigning::unsealed(
        br#"{"k":"v"}"#,
    ));
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
        .windows(b".mfbsign".len())
        .any(|window| window == b".mfbsign"));
    assert!(bytes
        .windows(br#"{"k":"v"}"#.len())
        .any(|window| window == br#"{"k":"v"}"#));
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
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    write_executable(dir.path(), "x86dp", "x86_64", LinuxFlavor::Glibc, &image)
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
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    let arch = if kind.starts_with("riscv") {
        "riscv64"
    } else if kind.ends_with("pc32") {
        "x86_64"
    } else {
        "aarch64"
    };
    let err = write_executable(dir.path(), "ub", arch, LinuxFlavor::Glibc, &image)
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
    expect_unbound(
        "riscv_got_hi20",
        "cannot bind external data symbol '_missing'",
    );
    expect_unbound(
        "riscv_got_lo12",
        "cannot bind external data symbol '_missing'",
    );
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
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    let err = write_executable(dir.path(), "undef", "aarch64", LinuxFlavor::Glibc, &image)
        .expect_err("undefined internal symbol must be rejected");
    assert!(err.contains("symbol '_ghost' does not resolve"));
}

#[test]
fn writes_glob_dat_glibc_elf() {
    let image = glob_dat_image("libc.so.6");
    let dir = std::path::PathBuf::from("tmp/globlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(&dir, "glob", "aarch64", LinuxFlavor::Glibc, &image)
        .expect("link glob_dat elf");
}

#[test]
fn writes_glob_dat_musl_elf() {
    let image = glob_dat_image("libc.musl-aarch64.so.1");
    let dir = std::path::PathBuf::from("tmp/globlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(&dir, "globmusl", "aarch64", LinuxFlavor::Musl, &image)
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
        signing_metadata: Some(crate::arch::image::ExecutableSigning::unsealed(
            br#"{"owner":"alice"}"#,
        )),
        rpaths: Vec::new(),
        debug_symbols: false,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "signed", "aarch64", LinuxFlavor::Glibc, &image)
        .expect("link signed elf");
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes
        .windows(b".mfbsign".len())
        .any(|window| window == b".mfbsign"));
    assert!(bytes
        .windows(br#"{"owner":"alice"}"#.len())
        .any(|window| window == br#"{"owner":"alice"}"#));
    assert_eq!(u16::from_le_bytes([bytes[60], bytes[61]]), 3);
    assert_eq!(u16::from_le_bytes([bytes[62], bytes[63]]), 2);
    image.signing_metadata = None;
    let unsigned = encode_static_elf("aarch64", 0, &image.text, &image.data, None, None);
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
    write_executable(&dir, "init", "aarch64", LinuxFlavor::Glibc, &image)
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
    let path = write_executable(&dir, "ver", "aarch64", LinuxFlavor::Glibc, &image)
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

// ===== RISC-V relocation pairing + reach =====

#[test]
fn riscv_pcrel_lo12_pairs_with_adjacent_auipc() {
    // The common case: `auipc rd,%hi` at 100, `addi rd,rd,%lo` at 104.
    let relocs = vec![
        riscv_reloc(100, "P", "riscv_pcrel_hi20"),
        riscv_reloc(104, "P", "riscv_pcrel_lo12"),
    ];
    assert_eq!(
        HiRelocIndex::build(&relocs).paired_auipc_offset(&relocs[1], "riscv_pcrel_hi20"),
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
        HiRelocIndex::build(&relocs).paired_auipc_offset(&relocs[1], "riscv_pcrel_hi20"),
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
        HiRelocIndex::build(&relocs).paired_auipc_offset(&relocs[1], "riscv_pcrel_hi20"),
        Ok(100)
    );
    assert_eq!(
        HiRelocIndex::build(&relocs).paired_auipc_offset(&relocs[4], "riscv_pcrel_hi20"),
        Ok(220)
    );
    // A lo12 with no preceding hi to its target is a hard error, not a silent
    // `offset - 4` guess.
    let orphan = vec![riscv_reloc(50, "Z", "riscv_pcrel_lo12")];
    assert!(HiRelocIndex::build(&orphan)
        .paired_auipc_offset(&orphan[0], "riscv_pcrel_hi20")
        .is_err());
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

/// bug-503: the project name is `Path::join`ed into `build/<name>-<flavor>.out`,
/// so a name carrying a path separator or `..` must be refused before any byte
/// is written — never resolved to a path outside `build/`.
#[test]
fn refuses_to_write_an_executable_under_a_traversing_name() {
    let image = x86_static_image();
    let dir = tempfile::tempdir().unwrap();
    let error = write_executable(dir.path(), "../evil", "x86_64", LinuxFlavor::Glibc, &image)
        .expect_err("a traversing project name must be rejected");
    assert!(error.contains("not a valid path component"), "{error}");
    assert!(
        !dir.path().join("evil-glibc.out").exists(),
        "an executable escaped build/ under a traversing name"
    );
    for name in [".hidden", "/tmp/evil", "a/b"] {
        assert!(
            write_executable(dir.path(), name, "x86_64", LinuxFlavor::Glibc, &image).is_err(),
            "{name}"
        );
    }
    assert!(
        !dir.path().join("build").exists(),
        "nothing may be created on refusal"
    );
}

// ---------------------------------------------------------------------------
// bug-552 — the `hi20`/`lo12` pairing index.
//
// The three tests above are the pairing RULE, and the rewrite leaves it alone:
// only how the nearest preceding `hi20` is found moved (a backward scan over the
// whole relocation list, O(R^2) per link, became a per-target index). These three
// cover what an index newly makes possible to get wrong.
// ---------------------------------------------------------------------------

#[test]
fn the_pairing_index_is_keyed_by_kind_as_well_as_target() {
    // A `got_hi20` for the SAME target sits nearer to the `lo12` than the
    // `pcrel_hi20` does. Keying the index by target alone would pair them, and
    // patch a data reference against a GOT slot's PC base.
    let relocs = vec![
        riscv_reloc(0x10, "msg", "riscv_pcrel_hi20"),
        riscv_reloc(0x48, "msg", "riscv_got_hi20"),
    ];
    let index = HiRelocIndex::build(&relocs);
    assert_eq!(
        index.paired_auipc_offset(
            &riscv_reloc(0x60, "msg", "riscv_pcrel_lo12"),
            "riscv_pcrel_hi20"
        ),
        Ok(0x10),
        "bug-552: a nearer got_hi20 must not be paired with a pcrel lo12"
    );
    assert_eq!(
        index.paired_auipc_offset(
            &riscv_reloc(0x60, "msg", "riscv_got_lo12"),
            "riscv_got_hi20"
        ),
        Ok(0x48),
        "bug-552: the GOT pair is indexed under its own kind"
    );
}

#[test]
fn the_pairing_index_does_not_assume_relocations_arrive_sorted() {
    // The encoder emits relocations in offset order today, and the backward scan
    // did not care either way because it took a max. A binary search does: over
    // an unsorted list it silently returns the WRONG `auipc` rather than failing,
    // so the index sorts, and this is what says so.
    let relocs = vec![
        riscv_reloc(0x80, "msg", "riscv_pcrel_hi20"),
        riscv_reloc(0x10, "msg", "riscv_pcrel_hi20"),
        riscv_reloc(0x40, "msg", "riscv_pcrel_hi20"),
    ];
    assert_eq!(
        HiRelocIndex::build(&relocs).paired_auipc_offset(
            &riscv_reloc(0x60, "msg", "riscv_pcrel_lo12"),
            "riscv_pcrel_hi20"
        ),
        Ok(0x40)
    );
}

#[test]
fn a_lo12_with_no_preceding_hi20_keeps_its_diagnostic() {
    // The only `auipc` for this target is AFTER the `lo12`. The scan reported
    // this; the index must report it with the same message rather than pairing
    // against something unrelated.
    let relocs = vec![riscv_reloc(0x80, "msg", "riscv_pcrel_hi20")];
    let error = HiRelocIndex::build(&relocs)
        .paired_auipc_offset(
            &riscv_reloc(0x60, "msg", "riscv_pcrel_lo12"),
            "riscv_pcrel_hi20",
        )
        .expect_err("a lo12 with no preceding hi20 must be an error");
    assert!(
        error.contains("has no paired riscv_pcrel_hi20"),
        "bug-552: the diagnostic must survive the index rewrite, got: {error}"
    );
}
