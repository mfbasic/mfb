use super::*;

mod built;
use crate::arch::image::{EncodedImage, EncodedImport, EncodedSection, EncodedSymbol, ImportKind};
use crate::debugger::arm64::AARCH64;
use crate::debugger::riscv64::RISCV64;
use crate::os::linux::flavor::LinuxFlavor;

/// A two-function AArch64 image importing `write` and `exit`, linked by the
/// in-tree Linux linker.
fn linked(debug: bool) -> Vec<u8> {
    linked_for("aarch64", [0xc0, 0x03, 0x5f, 0xd6], debug)
}

/// The same image for `arch`, its functions each the 4-byte `ret`.
fn linked_for(arch: &str, ret: [u8; 4], debug: bool) -> Vec<u8> {
    let image = EncodedImage {
        text: [ret, ret, ret].concat(),
        data: vec![0; 16],
        rodata_size: 0,
        symbols: vec![
            EncodedSymbol {
                name: "_start".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_mfb_fn_main".to_string(),
                section: EncodedSection::Text,
                offset: 4,
            },
            EncodedSymbol {
                name: "_mfb_rt_main_arena".to_string(),
                section: EncodedSection::Data,
                offset: 8,
            },
        ],
        relocations: Vec::new(),
        imports: ["write", "exit"]
            .iter()
            .map(|symbol| EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: symbol.to_string(),
                kind: ImportKind::Function,
                version: None,
            })
            .collect(),
        entry: "_start".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
        debug_symbols: debug,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = crate::os::linux::write_linked_executable(
        &dir.path().join("probe"),
        "probe",
        arch,
        LinuxFlavor::Glibc,
        &image,
    )
    .unwrap();
    std::fs::read(path).unwrap()
}

#[test]
fn a_debug_build_names_its_functions_data_imports_and_stubs() {
    let bytes = linked(true);
    let image = executable(&bytes, &AARCH64.elf).expect("an ELF the linker wrote");
    let elf = &image.elf;
    assert!(elf.position_independent);
    assert_eq!(elf.entry, 0x1000);
    assert_eq!(
        elf.interpreter.as_deref(),
        Some("/lib/ld-linux-aarch64.so.1")
    );
    assert_eq!(elf.needed, ["libc.so.6"]);
    assert_eq!(elf.program_headers, Some(64));
    let names = image
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function.location.clone()))
        .collect::<Vec<_>>();
    let slot = |name: &str| {
        elf.slots
            .iter()
            .find(|(index, _)| elf.dynamic[*index].name == name)
            .map(|(_, slot)| *slot)
    };
    assert_eq!(
        names,
        [
            (
                "_start",
                Location::Defined {
                    address: 0x1000,
                    size: 4
                }
            ),
            (
                "_mfb_fn_main",
                Location::Defined {
                    address: 0x1004,
                    size: 8
                }
            ),
            ("write", Location::Import(slot("write"))),
            ("exit", Location::Import(slot("exit"))),
        ]
    );
    assert!(slot("write").is_some() && slot("exit") == slot("write").map(|at| at + 8));
    assert_eq!(image.data.len(), 1);
    assert_eq!(image.data[0].0, "_mfb_rt_main_arena");
    assert_eq!(
        image.stubs,
        [
            ("write".to_string(), 0x100c, 12),
            ("exit".to_string(), 0x1018, 12)
        ]
    );
}

#[test]
fn a_build_without_debug_still_names_its_imports_through_pt_dynamic() {
    let bytes = linked(false);
    let image = executable(&bytes, &AARCH64.elf).unwrap();
    assert!(image.elf.symbols.is_empty());
    assert_eq!(
        image
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        ["write", "exit"]
    );
    assert_eq!(image.stubs.len(), 2);
    assert!(image.data.is_empty());
}

#[test]
fn the_code_and_defined_functions_are_listed_and_an_executable_has_no_plt() {
    let image = executable(&linked(true), &AARCH64.elf).unwrap();
    let elf = &image.elf;
    let code = elf.code();
    assert_eq!(code.len(), 1);
    assert!(code[0].0 == 0 && code[0].1 >= 0x1024, "{code:x?}");
    assert_eq!(
        elf.defined_functions(),
        [
            ("_start".to_string(), 0x1000, 4, false),
            ("_mfb_fn_main".to_string(), 0x1004, 8, false),
        ]
    );
    assert_eq!(elf.plt, None);
    assert_eq!(elf.plt_entry(0x1000), None);
}

#[test]
fn a_plt_entry_is_named_by_its_jump_slot_relocation() {
    let mut elf = read(&linked(false), &AARCH64.elf).unwrap();
    // A library's `.plt`: a 32-byte header, then 16 bytes per jump slot,
    // in the order of the `R_AARCH64_JUMP_SLOT` relocations.
    elf.plt = Some((0x2000, PLT_HEADER + 2 * PLT_ENTRY));
    let names = elf
        .plt_order
        .iter()
        .map(|index| elf.dynamic[*index].name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, ["write", "exit"]);
    assert_eq!(elf.plt_entry(0x2010), None);
    assert_eq!(elf.plt_entry(0x2024), Some((0x2020, "write".to_string())));
    assert_eq!(elf.plt_entry(0x2030), Some((0x2030, "exit".to_string())));
    assert_eq!(elf.plt_entry(0x2040), None);
}

#[test]
fn a_file_that_is_not_an_aarch64_elf_is_refused() {
    assert_eq!(read(b"not an elf at all", &AARCH64.elf), None);
    let mut bytes = linked(false);
    bytes[18] = 62; // EM_X86_64
    assert_eq!(read(&bytes, &AARCH64.elf), None);
    let truncated = &linked(true)[..200];
    assert_eq!(read(truncated, &AARCH64.elf), None);
}

/// `jalr zero, 0(ra)`: riscv64's `ret`.
const RISCV64_RET: [u8; 4] = [0x67, 0x80, 0x00, 0x00];

/// A riscv64 executable is read as an AArch64 one is: its imports through
/// `R_RISCV_JUMP_SLOT` relocations, and its 12-byte `auipc t3` / `ld t3` /
/// `jr t3` stubs.
#[test]
fn a_riscv64_executable_reads_its_symbols_and_stubs() {
    let bytes = linked_for("riscv64", RISCV64_RET, true);
    let image = executable(&bytes, &RISCV64.elf).expect("an ELF the linker wrote");
    let elf = &image.elf;
    assert_eq!(
        elf.interpreter.as_deref(),
        Some("/lib/ld-linux-riscv64-lp64d.so.1")
    );
    assert_eq!(elf.needed, ["libc.so.6"]);
    let slot = |name: &str| {
        elf.slots
            .iter()
            .find(|(index, _)| elf.dynamic[*index].name == name)
            .map(|(_, slot)| *slot)
    };
    assert_eq!(
        image
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.location.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "_start",
                Location::Defined {
                    address: 0x1000,
                    size: 4
                }
            ),
            (
                "_mfb_fn_main",
                Location::Defined {
                    address: 0x1004,
                    size: 8
                }
            ),
            ("write", Location::Import(slot("write"))),
            ("exit", Location::Import(slot("exit"))),
        ]
    );
    assert!(slot("write").is_some() && slot("exit") == slot("write").map(|at| at + 8));
    assert_eq!(
        image.stubs,
        [
            ("write".to_string(), 0x100c, 12),
            ("exit".to_string(), 0x1018, 12)
        ]
    );
}

#[test]
fn a_riscv64_elf_is_refused_as_aarch64_and_the_reverse() {
    let riscv64 = linked_for("riscv64", RISCV64_RET, false);
    assert!(read(&riscv64, &RISCV64.elf).is_some());
    assert_eq!(read(&riscv64, &AARCH64.elf), None);
    assert_eq!(read(&linked(false), &RISCV64.elf), None);
}
