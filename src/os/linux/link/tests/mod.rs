use super::*;
use crate::arch::image::{EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind};

mod elf_writing;
mod sections_and_symbols;

/// An empty image with `_main` as the entry: the base every test spreads over
/// with `..none()`, spelling only the fields it actually exercises (bug-335 C2).
fn none() -> EncodedImage {
    EncodedImage {
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
        debug_symbols: false,
    }
}

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
        ..none()
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
        ..none()
    }
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

/// The `(p_vaddr, p_memsz)` of a program header of `p_type`, if present.
fn phdr_range(bytes: &[u8], p_type: u32) -> Option<(u64, u64)> {
    let phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;
    (0..phnum).find_map(|i| {
        let base = phoff + i * 56;
        if u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) != p_type {
            return None;
        }
        let vaddr = u64::from_le_bytes(bytes[base + 16..base + 24].try_into().unwrap());
        let memsz = u64::from_le_bytes(bytes[base + 40..base + 48].try_into().unwrap());
        Some((vaddr, memsz))
    })
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
        ..none()
    }
}
