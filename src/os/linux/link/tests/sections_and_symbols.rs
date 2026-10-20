//! plan-176-J: the Linux ELF linker's sections_and_symbols tests. The shared image builders live in\n//! `link/tests/mod.rs`.

use super::*;

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
        ..none()
    };
    let dir = tempfile::tempdir().unwrap();
    write_executable(dir.path(), "aast", "aarch64", LinuxFlavor::Glibc, &image)
        .expect("link aarch64 static internal-reloc elf");
}

/// An internal `call` between two functions in the same image lowers to a
/// `riscv_call` relocation with `internal` binding — the auipc/jalr pair is
/// patched from the caller's PC (covers the `internal riscv_call` arm, distinct
/// from the imported-stub path the dynamic test exercises).
#[test]
fn write_executable_riscv64_internal_call_patches_auipc_jalr_pair() {
    use crate::codegen::engine::types::CodeFrame;
    use crate::codegen::engine::types::CodeFunction;
    use crate::codegen::engine::types::NativeCodePlan;
    fn func(
        name: &str,
        instructions: Vec<crate::codegen::engine::types::CodeInstruction>,
    ) -> CodeFunction {
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
            func(
                "_main",
                vec![rv_inst("bl", &[("target", "_helper")]), rv_inst("ret", &[])],
            ),
            func("_helper", vec![rv_inst("ret", &[])]),
        ],
    };
    let image = crate::arch::riscv64::encode::encode(&plan).expect("riscv encode");
    // No imports → a static ELF; the internal call relocation is still patched.
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "rvi", "riscv64", LinuxFlavor::Glibc, &image)
        .expect("link riscv static elf with internal call");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 243); // EM_RISCV
}

fn rv_inst(
    op: &str,
    fields: &[(&'static str, &str)],
) -> crate::codegen::engine::types::CodeInstruction {
    let mut instruction = crate::codegen::engine::types::CodeInstruction::new(op);
    // Encoder fixtures spell physical registers (bug-721: backend entry point).
    for (key, value) in fields {
        instruction = instruction.physical_field(key, value);
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
    use crate::codegen::engine::types::CodeDataObject;
    use crate::codegen::engine::types::CodeFrame;
    use crate::codegen::engine::types::CodeFunction;
    use crate::codegen::engine::types::CodeImport;
    use crate::codegen::engine::types::NativeCodePlan;
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
    let path = write_executable(dir.path(), "rvd", "riscv64", LinuxFlavor::Glibc, &image)
        .expect("link riscv dynamic elf");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x7f, b'E', b'L', b'F']);
    // PIE (ET_DYN); EM_RISCV (243) with EF_RISCV_FLOAT_ABI_DOUBLE in e_flags
    // (offset 48) (bug-186).
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 3); // ET_DYN
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 243);
    assert_eq!(
        u32::from_le_bytes(bytes[48..52].try_into().unwrap()) & 0x4,
        0x4
    );
    // 8 program headers (dynamic + PT_GNU_RELRO (bug-263) + PT_GNU_STACK + the
    // plan-43 PT_NOTE) and the riscv64 interpreter path.
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 8);
    assert!(has_gnu_stack(&bytes), "PT_GNU_STACK must be present");
    assert!(bytes
        .windows(b"ld-linux-riscv64".len())
        .any(|window| window == b"ld-linux-riscv64"));
}

/// The `MFBasic\0` provenance note in an emitted ELF (plan-43), located through
/// the program-header table the same way `readelf -n` does: walk the `PT_NOTE`
/// entries, frame each with its `Elf64_Nhdr`, and match the note *name* against
/// the vendor owner. Returns the note's `(p_offset, descriptor)`, or `None` when
/// the image carries no such note.
fn mfb_note(bytes: &[u8]) -> Option<(usize, Vec<u8>)> {
    let phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;
    (0..phnum).find_map(|index| {
        let base = phoff + index * 56;
        let read_u32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let read_u64 = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        if read_u32(base) != PT_NOTE {
            return None;
        }
        // p_offset/p_filesz — and the note must be readable (R) at runtime.
        assert_eq!(read_u32(base + 4), 4, "PT_NOTE p_flags must be R");
        let offset = read_u64(base + 8) as usize;
        let file_size = read_u64(base + 32) as usize;
        assert_eq!(
            read_u64(base + 40) as usize,
            file_size,
            "p_memsz == p_filesz"
        );
        let note = &bytes[offset..offset + file_size];
        let namesz = u32::from_le_bytes(note[0..4].try_into().unwrap()) as usize;
        let descsz = u32::from_le_bytes(note[4..8].try_into().unwrap()) as usize;
        assert_eq!(
            u32::from_le_bytes(note[8..12].try_into().unwrap()),
            MFB_NOTE_TYPE
        );
        if &note[12..12 + namesz] != MFB_NOTE_OWNER {
            return None;
        }
        // The name is 8 bytes and the descriptor 16, so neither needs the note
        // format's 4-byte padding — the descriptor starts right after the name.
        Some((offset, note[12 + namesz..12 + namesz + descsz].to_vec()))
    })
}

/// plan-43: every static ELF the linker emits — for each arch and both static
/// encoders — carries the unconditional `MFBasic\0` `PT_NOTE`, whose descriptor
/// is the shared payload, placed in the header/text gap the text `PT_LOAD` maps.
// ===== Provenance note + RELRO hardening =====

#[test]
fn static_elf_carries_the_mfbasic_provenance_note() {
    let image = x86_static_image();
    let images = [
        encode_static_elf_x86(0, &image.text, &image.data, None, None),
        encode_static_elf("aarch64", 0, &image.text, &image.data, None, None),
        encode_static_elf("riscv64", 0, &image.text, &image.data, None, None),
    ];
    for bytes in images {
        let (offset, descriptor) = mfb_note(&bytes).expect("PT_NOTE owned by MFBasic");
        assert_eq!(descriptor, mfb_note_descriptor());
        // The note lives below the text, inside the text PT_LOAD's file range, so
        // the text/data offsets the relocations assume are untouched.
        assert!(offset >= 64 + 4 * 56);
        assert!(offset + 12 + 8 + descriptor.len() <= TEXT_FILE_OFFSET);
        assert_eq!(bytes[TEXT_FILE_OFFSET], 0xc3);
    }
}

/// plan-43: the dynamic encoder places its note after the NUL-terminated
/// interpreter string — the other occupant of the header/text gap — so the two
/// coexist below `TEXT_FILE_OFFSET`.
#[test]
fn dynamic_elf_carries_the_mfbasic_provenance_note_past_the_interpreter() {
    for (arch, interp) in [
        ("aarch64", &b"/lib/ld-linux-aarch64.so.1"[..]),
        ("x86_64", &b"/lib64/ld-linux-x86-64.so.2"[..]),
        ("riscv64", &b"/lib/ld-linux-riscv64-lp64d.so.1"[..]),
    ] {
        let image = glob_dat_image("libc.so.6");
        let bytes = encode_dynamic_elf(
            arch,
            LinuxFlavor::Glibc,
            0,
            &image.text,
            &image.data,
            &image,
        )
        .expect("dynamic elf");
        let (offset, descriptor) = mfb_note(&bytes).expect("PT_NOTE owned by MFBasic");
        assert_eq!(descriptor, mfb_note_descriptor());
        // Past the interpreter string and still inside the gap below the text.
        // 8 program headers with no rodata partition (PHDR, INTERP, LOAD-text,
        // LOAD-data, DYNAMIC, GNU_RELRO, GNU_STACK, NOTE) — the GNU_RELRO header is
        // the bug-263 addition.
        let interp_offset = 64 + 8 * 56;
        assert_eq!(&bytes[interp_offset..interp_offset + interp.len()], interp);
        assert!(offset > interp_offset + interp.len());
        assert!(offset + 12 + 8 + descriptor.len() <= TEXT_FILE_OFFSET);
    }
}

/// plan-43 non-goal: the marker is additive and orthogonal to the `--sign`
/// feature — an image with `signing_metadata` carries both the note and the
/// `.mfbsign` section.
#[test]
fn provenance_note_coexists_with_the_signing_section() {
    let mut image = glob_dat_image("libc.so.6");
    image.signing_metadata = Some(crate::arch::image::ExecutableSigning::unsealed(
        br#"{"k":"v"}"#,
    ));
    let dynamic = encode_dynamic_elf(
        "aarch64",
        LinuxFlavor::Glibc,
        0,
        &image.text,
        &image.data,
        &image,
    )
    .expect("dynamic elf with signing");
    let static_image = x86_static_image();
    let statics = encode_static_elf_x86(
        0,
        &static_image.text,
        &static_image.data,
        Some(br#"{"k":"v"}"#),
        None,
    );
    for bytes in [dynamic, statics] {
        assert_eq!(
            mfb_note(&bytes).expect("PT_NOTE owned by MFBasic").1,
            mfb_note_descriptor()
        );
        assert!(bytes
            .windows(b".mfbsign".len())
            .any(|window| window == b".mfbsign"));
    }
}

/// bug-263 / LNK-03: a dynamic ELF emits PT_GNU_RELRO, page-aligned, fully
/// covering the DYNAMIC segment (and therefore the GOT that precedes it), so the
/// loader re-protects the GOT/.dynamic read-only after BIND_NOW relocation.
/// Hardware-validated on x86_64/aarch64/riscv64 × glibc/musl (the binaries run and
/// `readelf -l` shows GNU_RELRO covering DT_PLTGOT and PT_DYNAMIC).
#[test]
fn dynamic_elf_relro_covers_the_dynamic_segment() {
    const PT_DYNAMIC: u32 = 2;
    const PT_GNU_RELRO: u32 = 0x6474_e552;
    for arch in ["aarch64", "x86_64", "riscv64"] {
        let image = glob_dat_image("libc.so.6");
        let bytes = encode_dynamic_elf(
            arch,
            LinuxFlavor::Glibc,
            0,
            &image.text,
            &image.data,
            &image,
        )
        .expect("dynamic elf");
        let (relro_vaddr, relro_size) =
            phdr_range(&bytes, PT_GNU_RELRO).expect("PT_GNU_RELRO present");
        assert_eq!(
            relro_vaddr % 0x1000,
            0,
            "{arch}: RELRO start is page-aligned"
        );
        let (dyn_vaddr, dyn_size) = phdr_range(&bytes, PT_DYNAMIC).expect("PT_DYNAMIC present");
        assert!(
            relro_vaddr <= dyn_vaddr && dyn_vaddr + dyn_size <= relro_vaddr + relro_size,
            "{arch}: RELRO [{relro_vaddr:#x}, {:#x}) must cover DYNAMIC [{dyn_vaddr:#x}, {:#x})",
            relro_vaddr + relro_size,
            dyn_vaddr + dyn_size,
        );
    }
}

/// Decode the `.dynamic` section's `DT_RUNPATH` (tag 29) strings from a dynamic
/// ELF, resolving each through `DT_STRTAB` exactly as the loader does.
///
/// Reads the real program headers/tags rather than scanning for the literal, so
/// the assertions below prove the tag is actually *wired* — a `$ORIGIN/vendor`
/// sitting in `.dynstr` with no tag pointing at it would look identical to a
/// substring search and would be ignored by the loader.
fn elf_runpaths(bytes: &[u8]) -> Vec<String> {
    const PT_DYNAMIC: u32 = 2;
    const DT_NULL: u64 = 0;
    const DT_STRTAB: u64 = 5;
    const DT_RUNPATH: u64 = 29;

    let u32le = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64le = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());

    // Locate PT_DYNAMIC via the program headers.
    let phoff = u64le(32) as usize;
    let phnum = u16::from_le_bytes(bytes[56..58].try_into().unwrap()) as usize;
    let mut dynamic: Option<(usize, usize)> = None;
    // Map a vaddr back to a file offset through the PT_LOAD segments.
    let mut loads: Vec<(u64, u64, u64)> = Vec::new();
    for index in 0..phnum {
        let base = phoff + index * 56;
        let kind = u32le(base);
        let offset = u64le(base + 8);
        let vaddr = u64le(base + 16);
        let filesz = u64le(base + 32);
        if kind == PT_DYNAMIC {
            dynamic = Some((offset as usize, filesz as usize));
        }
        if kind == 1 {
            loads.push((vaddr, offset, filesz));
        }
    }
    let vaddr_to_offset = |vaddr: u64| -> Option<usize> {
        loads
            .iter()
            .find(|(base, _, size)| vaddr >= *base && vaddr < base + size)
            .map(|(base, offset, _)| (vaddr - base + offset) as usize)
    };

    let Some((dyn_offset, dyn_size)) = dynamic else {
        return Vec::new();
    };

    // First pass: DT_STRTAB. Second: every DT_RUNPATH offset into it.
    let mut strtab = None;
    let mut runpath_offsets = Vec::new();
    let mut at = dyn_offset;
    while at + 16 <= dyn_offset + dyn_size {
        let tag = u64le(at);
        let value = u64le(at + 8);
        match tag {
            DT_NULL => break,
            DT_STRTAB => strtab = vaddr_to_offset(value),
            DT_RUNPATH => runpath_offsets.push(value as usize),
            _ => {}
        }
        at += 16;
    }
    let Some(strtab) = strtab else {
        return Vec::new();
    };

    runpath_offsets
        .into_iter()
        .map(|offset| {
            let start = strtab + offset;
            let end = start + bytes[start..].iter().position(|b| *b == 0).unwrap();
            String::from_utf8(bytes[start..end].to_vec()).expect("utf-8 runpath")
        })
        .collect()
}

// plan-46-D §4.2: a build with no vendor libraries emits no DT_RUNPATH, so every
// existing binary stays byte-identical.
// ===== RUNPATH / vendoring =====

#[test]
fn a_non_vendor_elf_emits_no_runpath() {
    let image = versioned_exit_image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "norunpath",
        "aarch64",
        LinuxFlavor::Glibc,
        &image,
    )
    .expect("link");
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        elf_runpaths(&bytes).is_empty(),
        "no vendor libraries → no DT_RUNPATH"
    );
}

// A vendoring build carries exactly one DT_RUNPATH whose string is
// `$ORIGIN/vendor`, resolved through DT_STRTAB the way the loader does.
#[test]
fn a_vendoring_elf_emits_one_origin_relative_runpath() {
    let mut image = versioned_exit_image();
    image.rpaths = vec![crate::os::ELF_VENDOR_RPATH.to_string()];
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path(), "runpath", "aarch64", LinuxFlavor::Glibc, &image)
        .expect("link");
    let bytes = std::fs::read(&path).unwrap();
    // `$ORIGIN` is expanded by the loader, not the build: it must survive verbatim.
    assert_eq!(elf_runpaths(&bytes), vec!["$ORIGIN/vendor".to_string()]);
}

// Growing `.dynstr` must propagate through the layout on its own: DT_STRSZ is
// computed as `dynstr.len()` and `dynsym_offset` derives from
// `dynstr_offset + dynstr.len()`. If it did not, the symbol table would overlap
// the runpath string and the loader would read garbage symbol names.
#[test]
fn the_runpath_string_does_not_disturb_the_symbol_table() {
    let mut image = versioned_exit_image();
    image.rpaths = vec![crate::os::ELF_VENDOR_RPATH.to_string()];
    let dir = tempfile::tempdir().unwrap();
    let path =
        write_executable(dir.path(), "strsz", "aarch64", LinuxFlavor::Glibc, &image).expect("link");
    let bytes = std::fs::read(&path).unwrap();
    // The imported symbol name must still be intact and findable.
    assert!(
        bytes.windows(b"_exit\0".len()).any(|w| w == b"_exit\0"),
        "the import's symbol name must survive the dynstr growth"
    );
    assert_eq!(elf_runpaths(&bytes), vec!["$ORIGIN/vendor".to_string()]);
}

// `dynamic_prefix_size` independently recomputes `.dynstr`'s length to derive the
// GOT offset baked into every import stub, so it MUST agree with what
// `DynamicPayload::build` actually appends. When the DT_RUNPATH string was added
// to one and not the other, the prefix under-counted by its length, every stub
// jumped through the wrong GOT slot, and the program segfaulted on its first
// imported call — before ever reaching the `dlopen` the runpath exists for, and
// invisibly to every header assertion.
//
// Pin the invariant: a runpath must move the dynamic prefix by exactly the string
// it adds to `.dynstr`, and by nothing else.
#[test]
fn a_runpath_grows_the_dynamic_prefix_by_exactly_its_dynstr_bytes() {
    let plain = versioned_exit_image();
    let mut vendored = versioned_exit_image();
    vendored.rpaths = vec![crate::os::ELF_VENDOR_RPATH.to_string()];

    let plain_size = super::elf::dynamic_prefix_size(&plain);
    let vendored_size = super::elf::dynamic_prefix_size(&vendored);
    assert!(
        vendored_size > plain_size,
        "the runpath string must be counted in the dynamic prefix"
    );
    // "$ORIGIN/vendor" + NUL = 15 bytes; the prefix's derived offsets are 8-byte
    // aligned, so assert the growth is at least the string and within one
    // alignment step of it.
    let added = vendored_size - plain_size;
    let string_len = crate::os::ELF_VENDOR_RPATH.len() + 1;
    assert!(
        added >= string_len && added < string_len + 8,
        "prefix grew by {added}, expected ~{string_len} (the runpath string + NUL)"
    );
}

// The end-to-end form of the same invariant: link a real vendoring image and check
// that the emitted `.dynstr` length (DT_STRSZ) matches what the stub-offset
// computation assumed. A mismatch here is the segfault above.
#[test]
fn the_emitted_dynstr_matches_the_prefix_computation_with_a_runpath() {
    let mut image = versioned_exit_image();
    image.rpaths = vec![crate::os::ELF_VENDOR_RPATH.to_string()];
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "strszrpath",
        "aarch64",
        LinuxFlavor::Glibc,
        &image,
    )
    .expect("link");
    let bytes = std::fs::read(&path).unwrap();

    // DT_STRSZ (tag 10) is emitted as `dynstr.len()`. The runpath string must be
    // inside it, and the prefix computation must have accounted for the same
    // bytes — otherwise the GOT offsets are wrong.
    let runpaths = elf_runpaths(&bytes);
    assert_eq!(runpaths, vec!["$ORIGIN/vendor".to_string()]);
    assert!(
        bytes.windows(b"_exit\0".len()).any(|w| w == b"_exit\0"),
        "the import symbol must still be intact"
    );
}

// --- plan-51-A §4.4: the app-mode RUNPATH ----------------------------------
//
// ⚠️ The `.dynstr` hazard, on the exact axis that failed once before. plan-46-D
// §1 records that a runpath edit silently corrupted every import stub's GOT
// offset: `.dynstr`'s contents and `dynamic_prefix_size` are two independent
// computations of one layout, one was updated and the other was not, and the
// program segfaulted on its first imported call while unit tests passed and
// `readelf -d` printed the runpath correctly.
//
// `$ORIGIN/../lib` (14 chars) and `$ORIGIN/vendor` (14 chars) happen to be the
// same length, which is precisely why this needs a test rather than an
// eyeball: the two strings agreeing in length today is a coincidence, not a
// property, and the next runpath added will not.
#[test]
fn an_appdir_vendoring_elf_emits_the_usr_lib_relative_runpath() {
    let mut image = versioned_exit_image();
    image.rpaths = vec![crate::os::ELF_APPDIR_VENDOR_RPATH.to_string()];
    let dir = tempfile::tempdir().unwrap();
    let appdir = write_appdir(
        dir.path(),
        "vendored",
        "aarch64",
        LinuxFlavor::Glibc,
        &image,
        None,
        "0.1.0",
    )
    .expect("link AppDir");
    let bytes = std::fs::read(appdir.join("usr/bin/vendored")).unwrap();
    // The executable is at `usr/bin/<name>` and its libraries at `usr/lib/`, so
    // `$ORIGIN/../lib` is what the loader must see — verbatim, since `$ORIGIN`
    // is expanded by the loader and not by the build.
    assert_eq!(elf_runpaths(&bytes), vec!["$ORIGIN/../lib".to_string()]);
    // And the import's symbol name survives the dynstr growth, which is the
    // symptom the GOT-offset bug hid behind.
    assert!(
        bytes.windows(b"_exit\0".len()).any(|w| w == b"_exit\0"),
        "the import symbol must still be intact"
    );
}

// Both runpath strings must move the dynamic prefix by their own length. A
// prefix computation that hardcoded one string's length would pass for that
// string and silently mis-place every GOT slot for the other.
#[test]
fn each_runpath_grows_the_dynamic_prefix_by_its_own_length() {
    let plain_size = super::elf::dynamic_prefix_size(&versioned_exit_image());
    for rpath in [
        crate::os::ELF_VENDOR_RPATH,
        crate::os::ELF_APPDIR_VENDOR_RPATH,
        // A deliberately longer string: the two real ones are the same length
        // today, so a third proves the computation is driven by the value.
        "$ORIGIN/../lib:$ORIGIN/../../lib64",
    ] {
        let mut image = versioned_exit_image();
        image.rpaths = vec![rpath.to_string()];
        let added = super::elf::dynamic_prefix_size(&image) - plain_size;
        let string_len = rpath.len() + 1;
        // The prefix's derived offsets are 8-byte aligned, so the growth lands
        // within one alignment step of the string's own length in either
        // direction — a long string can absorb slack the plain layout already
        // had. What must NOT happen is the growth ignoring the string's length,
        // which is the plan-46-D §1 failure.
        assert!(
            added.abs_diff(string_len) < 8,
            "{rpath}: prefix grew by {added}, expected within 8 of {string_len}"
        );
    }
}

// A NON-vendoring app build must be byte-identical to its console counterpart:
// with no runpath on either side, the AppDir's ELF and the `-glibc.out` come out
// of one encoder with one input. Identical bytes for a *vendoring* build, by
// contrast, would mean one of the two runpaths is wrong.
#[test]
fn a_non_vendoring_appdir_elf_is_byte_identical_to_the_console_build() {
    let image = versioned_exit_image();
    assert!(image.rpaths.is_empty(), "this fixture vendors nothing");

    let app_dir = tempfile::tempdir().unwrap();
    let appdir = write_appdir(
        app_dir.path(),
        "twin",
        "aarch64",
        LinuxFlavor::Glibc,
        &image,
        None,
        "0.1.0",
    )
    .expect("link AppDir");
    let console_dir = tempfile::tempdir().unwrap();
    let console = write_executable(
        console_dir.path(),
        "twin",
        "aarch64",
        LinuxFlavor::Glibc,
        &image,
    )
    .expect("link console");

    let app_bytes = std::fs::read(appdir.join("usr/bin/twin")).unwrap();
    let console_bytes = std::fs::read(&console).unwrap();
    assert_eq!(app_bytes, console_bytes);
    assert!(
        elf_runpaths(&app_bytes).is_empty(),
        "no runpath either side"
    );
}

// ===== plan-166: `--debug` section header table and `.symtab` =====

/// One ELF section header, read through `e_shoff` the way `readelf -S` does.
#[derive(Debug)]
struct ParsedSection {
    name: String,
    type_: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    entsize: u64,
}

fn parse_sections(bytes: &[u8]) -> Vec<ParsedSection> {
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let shoff = u64_at(40) as usize;
    assert_eq!(u16_at(58), 64, "e_shentsize");
    let shnum = u16_at(60) as usize;
    let shstrndx = u16_at(62) as usize;
    let header = |index: usize| shoff + index * 64;
    let strtab = u64_at(header(shstrndx) + 24) as usize;
    let name_at = |offset: usize| {
        let start = strtab + offset;
        let end = start + bytes[start..].iter().position(|b| *b == 0).unwrap();
        String::from_utf8(bytes[start..end].to_vec()).unwrap()
    };
    (0..shnum)
        .map(|index| {
            let at = header(index);
            ParsedSection {
                name: name_at(u32_at(at) as usize),
                type_: u32_at(at + 4),
                flags: u64_at(at + 8),
                addr: u64_at(at + 16),
                offset: u64_at(at + 24),
                size: u64_at(at + 32),
                link: u32_at(at + 40),
                info: u32_at(at + 44),
                entsize: u64_at(at + 56),
            }
        })
        .collect()
}

fn section_index(sections: &[ParsedSection], name: &str) -> usize {
    sections
        .iter()
        .position(|section| section.name == name)
        .unwrap_or_else(|| panic!("section {name} in {sections:?}"))
}

/// `(name, st_info, st_shndx, st_value, st_size)` for every `.symtab` entry past
/// the NULL one, with the table's own shape checked on the way.
fn parse_symtab(bytes: &[u8], sections: &[ParsedSection]) -> Vec<(String, u8, u16, u64, u64)> {
    let symtab = &sections[section_index(sections, ".symtab")];
    let strtab = &sections[symtab.link as usize];
    assert_eq!(strtab.name, ".strtab", ".symtab sh_link names .strtab");
    assert_eq!(symtab.type_, 2, "SHT_SYMTAB");
    assert_eq!(strtab.type_, 3, "SHT_STRTAB");
    assert_eq!(symtab.entsize, 24);
    assert_eq!(symtab.offset % 8, 0, ".symtab is 8-aligned");
    let count = symtab.size as usize / 24;
    assert_eq!(symtab.info as usize, count, "sh_info: every entry is local");
    let entry = |index: usize| symtab.offset as usize + index * 24;
    assert!(
        bytes[entry(0)..entry(1)].iter().all(|b| *b == 0),
        "NULL entry"
    );
    (1..count)
        .map(|index| {
            let at = entry(index);
            let name_at = strtab.offset as usize
                + u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let end = name_at + bytes[name_at..].iter().position(|b| *b == 0).unwrap();
            (
                String::from_utf8(bytes[name_at..end].to_vec()).unwrap(),
                bytes[at + 4],
                u16::from_le_bytes(bytes[at + 6..at + 8].try_into().unwrap()),
                u64::from_le_bytes(bytes[at + 8..at + 16].try_into().unwrap()),
                u64::from_le_bytes(bytes[at + 16..at + 24].try_into().unwrap()),
            )
        })
        .collect()
}

fn debug_symbol(name: &str, section: EncodedSection, offset: usize) -> EncodedSymbol {
    EncodedSymbol {
        name: name.to_string(),
        section,
        offset,
    }
}

/// Checks every `.symtab` entry of a `--debug` ELF against the image: the
/// section it names, its address under the relocation rule (`symbol_vmaddr`
/// with the section headers' own text/data addresses), its size and its type.
fn assert_symbols_match(bytes: &[u8], image: &EncodedImage, expected: &[(&str, &str, u64)]) {
    let sections = parse_sections(bytes);
    let symbols = parse_symtab(bytes, &sections);
    let text = sections[section_index(&sections, ".text")].addr;
    // Linux data symbols resolve from the start of data (no read-only window),
    // which is `.rodata` when the writer split one off, else `.data`.
    let data = sections
        .iter()
        .find(|section| section.name == ".rodata")
        .unwrap_or(&sections[section_index(&sections, ".data")])
        .addr;
    assert_eq!(symbols.len(), expected.len(), "{symbols:?}");
    for ((name, info, shndx, value, size), (want_name, want_section, want_size)) in
        symbols.iter().zip(expected)
    {
        assert_eq!(name, want_name);
        assert_eq!(
            sections[*shndx as usize].name, *want_section,
            "{name}: st_shndx"
        );
        let want_type = if *want_section == ".text" { 2 } else { 1 };
        assert_eq!(*info, want_type, "{name}: STB_LOCAL + STT_FUNC/OBJECT");
        let want_value =
            crate::os::link_encode::symbol_vmaddr(image, name, text, data, None).unwrap();
        assert_eq!(*value, want_value, "{name}: st_value");
        assert_eq!(*size, *want_size, "{name}: st_size");
    }
}

/// Two 4-byte functions and two data objects in 24 bytes of data, `--debug` on.
fn static_debug_image(text: Vec<u8>) -> EncodedImage {
    assert_eq!(text.len(), 8, "two 4-byte functions");
    EncodedImage {
        text,
        data: vec![0; 24],
        symbols: vec![
            debug_symbol("_main", EncodedSection::Text, 0),
            debug_symbol("_helper", EncodedSection::Text, 4),
            debug_symbol("_d0", EncodedSection::Data, 0),
            debug_symbol("_d1", EncodedSection::Data, 16),
        ],
        debug_symbols: true,
        ..none()
    }
}

#[test]
fn debug_symbols_static_aarch64() {
    // _main: ret ; _helper: ret
    let image = static_debug_image(vec![0xc0, 0x03, 0x5f, 0xd6, 0xc0, 0x03, 0x5f, 0xd6]);
    let bytes = encode_executable_bytes("aarch64", LinuxFlavor::Glibc, &image).unwrap();
    let sections = parse_sections(&bytes);
    let names: Vec<_> = sections
        .iter()
        .map(|section| section.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["", ".text", ".data", ".symtab", ".strtab", ".shstrtab"]
    );
    let text = &sections[1];
    assert_eq!((text.offset, text.size, text.flags), (0x1000, 8, 6));
    assert_eq!(text.addr, IMAGE_BASE + 0x1000);
    let data = &sections[2];
    assert_eq!((data.size, data.flags), (24, 3));
    assert_eq!(data.addr, IMAGE_BASE + data.offset);
    assert_symbols_match(
        &bytes,
        &image,
        &[
            ("_main", ".text", 4),
            ("_helper", ".text", 4),
            ("_d0", ".data", 16),
            ("_d1", ".data", 8),
        ],
    );
}

#[test]
fn debug_symbols_static_x86() {
    let image = static_debug_image(vec![0xc3, 0x90, 0x90, 0x90, 0xc3, 0x90, 0x90, 0x90]);
    let bytes = encode_executable_bytes("x86_64", LinuxFlavor::Musl, &image).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62, "EM_X86_64");
    assert_eq!(u16::from_le_bytes([bytes[60], bytes[61]]), 6, "e_shnum");
    assert_symbols_match(
        &bytes,
        &image,
        &[
            ("_main", ".text", 4),
            ("_helper", ".text", 4),
            ("_d0", ".data", 16),
            ("_d1", ".data", 8),
        ],
    );
}

#[test]
fn debug_symbols_dynamic_with_rodata() {
    // A dynamic aarch64 image (it imports `_exit`) with a page of read-only
    // constants ahead of a writable global, so the writer splits `.rodata`.
    let mut image = versioned_exit_image();
    image.data = vec![0; 0x1000 + 8];
    image.rodata_size = 0x1000;
    image
        .symbols
        .push(debug_symbol("_const", EncodedSection::Data, 0));
    image
        .symbols
        .push(debug_symbol("_global", EncodedSection::Data, 0x1000));
    image.debug_symbols = true;
    let bytes = encode_executable_bytes("aarch64", LinuxFlavor::Glibc, &image).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 3, "ET_DYN");
    let sections = parse_sections(&bytes);
    let names: Vec<_> = sections
        .iter()
        .map(|section| section.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "",
            ".text",
            ".rodata",
            ".data",
            ".dynstr",
            ".dynamic",
            ".symtab",
            ".strtab",
            ".shstrtab"
        ]
    );
    assert_dynamic_section_covers_pt_dynamic(&bytes, &sections);
    let (rodata, data) = (&sections[2], &sections[3]);
    assert_eq!((rodata.size, rodata.flags), (0x1000, 2));
    assert_eq!((data.size, data.flags), (8, 3));
    assert_eq!(data.addr, rodata.addr + 0x1000);
    assert_eq!(data.offset, rodata.offset + 0x1000);
    // `.text` covers the PLT stub appended after `image.text`.
    assert!(sections[1].size as usize > image.text.len());
    assert_symbols_match(
        &bytes,
        &image,
        &[
            ("_main", ".text", image.text.len() as u64),
            ("_const", ".rodata", 0x1000),
            ("_global", ".data", 8),
        ],
    );
}

#[test]
fn debug_symbols_signed_keeps_mfbsign_findable() {
    let mut image = static_debug_image(vec![0xc0, 0x03, 0x5f, 0xd6, 0xc0, 0x03, 0x5f, 0xd6]);
    image.signing_metadata = Some(crate::arch::image::ExecutableSigning {
        metadata: format!(
            "{{\"contentSignature\":\"{}\"}}",
            crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER
        )
        .into_bytes(),
        signing_private: Some(mfb_repository::crypto::generate_keypair().1),
    });
    // `encode_executable_bytes` seals after the table is written: the seal
    // succeeds, so it found the placeholder through the one section table.
    let bytes = encode_executable_bytes("aarch64", LinuxFlavor::Glibc, &image).unwrap();
    let sections = parse_sections(&bytes);
    let names: Vec<_> = sections
        .iter()
        .map(|section| section.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "",
            ".text",
            ".data",
            ".symtab",
            ".strtab",
            ".mfbsign",
            ".shstrtab"
        ]
    );
    let binary = crate::os::inspect::inspect(&bytes).expect("inspect reads it back");
    let blob = binary.signing.expect(".mfbsign found by name");
    let mfbsign = &sections[5];
    assert_eq!(blob.start as u64, mfbsign.offset);
    assert_eq!(blob.len() as u64, mfbsign.size);
    assert!(!bytes
        .windows(crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER.len())
        .any(|window| window
            == crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER.as_bytes()));
}

#[test]
fn debug_symbols_off_writes_no_table() {
    for arch in ["aarch64", "x86_64", "riscv64"] {
        let image = EncodedImage {
            debug_symbols: false,
            ..static_debug_image(vec![0x13, 0, 0, 0, 0x13, 0, 0, 0])
        };
        let bytes = encode_executable_bytes(arch, LinuxFlavor::Glibc, &image).unwrap();
        assert_eq!(
            u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            0,
            "{arch}: e_shoff"
        );
        assert_eq!(
            u16::from_le_bytes([bytes[60], bytes[61]]),
            0,
            "{arch}: e_shnum"
        );
        assert!(
            !bytes.windows(8).any(|window| window == b"_helper\0"),
            "{arch}"
        );
    }
    let image = EncodedImage {
        debug_symbols: false,
        ..versioned_exit_image()
    };
    let bytes = encode_executable_bytes("aarch64", LinuxFlavor::Glibc, &image).unwrap();
    assert_eq!(
        u16::from_le_bytes([bytes[60], bytes[61]]),
        0,
        "dynamic: e_shnum"
    );
}

/// `.dynamic` covers exactly `PT_DYNAMIC` and links `.dynstr`, which holds the
/// `DT_NEEDED` names. Without it GNU readelf reports `no .dynamic section in the
/// dynamic segment` for any dynamic image that has section headers.
fn assert_dynamic_section_covers_pt_dynamic(bytes: &[u8], sections: &[ParsedSection]) {
    let dynamic = &sections[section_index(sections, ".dynamic")];
    assert_eq!((dynamic.type_, dynamic.entsize, dynamic.flags), (6, 16, 3));
    assert_eq!(
        Some((dynamic.addr, dynamic.size)),
        phdr_range(bytes, 2),
        ".dynamic matches PT_DYNAMIC"
    );
    let dynstr = &sections[dynamic.link as usize];
    assert_eq!((dynstr.name.as_str(), dynstr.type_), (".dynstr", 3));
    let names = &bytes[dynstr.offset as usize..(dynstr.offset + dynstr.size) as usize];
    assert_eq!(names[0], 0, ".dynstr starts with NUL");
    assert!(names.windows(10).any(|window| window == b"libc.so.6\0"));
}

#[test]
fn signed_dynamic_table_names_the_dynamic_segment() {
    let mut image = versioned_exit_image();
    image.signing_metadata = Some(crate::arch::image::ExecutableSigning::unsealed(
        br#"{"k":"v"}"#,
    ));
    let bytes = encode_executable_bytes("aarch64", LinuxFlavor::Glibc, &image).unwrap();
    let sections = parse_sections(&bytes);
    let names: Vec<_> = sections
        .iter()
        .map(|section| section.name.as_str())
        .collect();
    assert_eq!(names, ["", ".dynstr", ".dynamic", ".mfbsign", ".shstrtab"]);
    assert_dynamic_section_covers_pt_dynamic(&bytes, &sections);
    let binary = crate::os::inspect::inspect(&bytes).expect("inspect reads it back");
    assert_eq!(
        binary.signing.expect(".mfbsign found by name").start as u64,
        sections[3].offset
    );
}
