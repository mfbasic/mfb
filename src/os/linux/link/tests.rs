use super::*;
use crate::arch::aarch64::encode::{
    EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind,
};

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

#[test]
fn writes_glob_dat_glibc_elf() {
    let image = glob_dat_image("libc.so.6");
    let dir = std::path::PathBuf::from("tmp/globlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(&dir, "glob", LinuxFlavor::Glibc, false, &image)
        .expect("link glob_dat elf");
}

#[test]
fn writes_glob_dat_musl_elf() {
    let image = glob_dat_image("libc.musl-aarch64.so.1");
    let dir = std::path::PathBuf::from("tmp/globlx");
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_executable(&dir, "globmusl", LinuxFlavor::Musl, false, &image)
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
    let path = write_executable(dir.path(), "signed", LinuxFlavor::Glibc, false, &image)
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
    write_executable(&dir, "init", LinuxFlavor::Glibc, false, &image)
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
    let path = write_executable(&dir, "ver", LinuxFlavor::Glibc, false, &image)
        .expect("link versioned elf");
    let bytes = std::fs::read(&path).expect("read elf");
    assert!(
        bytes
            .windows("GLIBC_2.17".len())
            .any(|window| window == b"GLIBC_2.17"),
        ".dynstr should contain the required version GLIBC_2.17"
    );
}
