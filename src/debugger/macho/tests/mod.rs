use super::*;

mod patched;
use crate::arch::image::{EncodedImage, EncodedImport, EncodedSection, EncodedSymbol, ImportKind};

/// A linked Mach-O whose code defines `symbols` (at 4-byte steps) and
/// imports `imports`, with or without its debug symbol table.
fn linked(symbols: &[&str], imports: &[&str], debug: bool) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let mut text = Vec::new();
    for _ in symbols {
        text.extend([0xc0, 0x03, 0x5f, 0xd6]);
    }
    let image = EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: symbols
            .iter()
            .enumerate()
            .map(|(at, name)| EncodedSymbol {
                name: name.to_string(),
                section: EncodedSection::Text,
                offset: at * 4,
            })
            .collect(),
        relocations: Vec::new(),
        imports: imports
            .iter()
            .map(|symbol| EncodedImport {
                library: "libSystem".to_string(),
                symbol: symbol.to_string(),
                kind: ImportKind::Function,
                version: None,
            })
            .collect(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
        debug_symbols: debug,
    };
    let path = crate::os::macos::write_linked_executable(dir.path(), "probe", &image)
        .expect("link Mach-O");
    std::fs::read(path).unwrap()
}

fn summary(image: &Image) -> Vec<(String, bool)> {
    image
        .functions
        .iter()
        .map(|function| {
            (
                function.name.clone(),
                matches!(function.location, Location::Defined { .. }),
            )
        })
        .collect()
}

#[test]
fn a_debug_build_lists_its_functions_then_every_import_record() {
    let bytes = linked(
        &["_main", "_mfb_fn_main", "_mfb_rt_print"],
        &["_malloc", "_free", "_malloc"],
        true,
    );
    let image = read(&bytes).expect("a well-formed Mach-O");
    assert_eq!(image.text_address, 0x1_0000_0000);
    // The entry is `_main`, the first function.
    let main = image
        .functions
        .iter()
        .find_map(|function| match function.location {
            Location::Defined { address, .. } if function.name == "_main" => Some(address),
            _ => None,
        });
    assert_eq!(image.entry, main);
    assert_eq!(
        summary(&image),
        [
            ("_main", true),
            ("_mfb_fn_main", true),
            ("_mfb_rt_print", true),
            ("_malloc", false),
            ("_free", false),
            ("_malloc", false),
        ]
        .map(|(name, defined)| (name.to_string(), defined))
    );
    // Code addresses are the symbols' offsets into `__text`, in order.
    let addresses = image
        .functions
        .iter()
        .filter_map(|function| match function.location {
            Location::Defined { address, size } => Some((address, size)),
            Location::Import(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(addresses[1].0 - addresses[0].0, 4);
    assert_eq!(addresses[2].0 - addresses[1].0, 4);
    // Each function runs to the next, and the last to the end of `__text`.
    assert!(
        addresses.iter().all(|(_, size)| *size == 4),
        "{addresses:?}"
    );
    // Every import record has its own slot, which the file leaves unbound.
    let slots = image
        .functions
        .iter()
        .filter_map(|function| match &function.location {
            Location::Import(slot) => Some(slot.as_ref().expect("an import slot")),
            Location::Defined { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(slots.len(), 3);
    // One 12-byte stub per import record, in order.
    assert_eq!(
        image
            .stubs
            .iter()
            .map(|(name, _, size)| (name.as_str(), *size))
            .collect::<Vec<_>>(),
        [("_malloc", 12), ("_free", 12), ("_malloc", 12)]
    );
    assert!(slots
        .windows(2)
        .all(|pair| pair[0].address != pair[1].address));
    assert!(slots.iter().all(|slot| slot.unbound == [0; 8]));
}

#[test]
fn a_normal_build_lists_only_its_imports() {
    let bytes = linked(&["_main", "_mfb_fn_main"], &["_exit"], false);
    let image = read(&bytes).unwrap();
    assert_eq!(summary(&image), [("_exit".to_string(), false)]);
}

#[test]
fn data_symbols_are_not_functions() {
    let dir = tempfile::tempdir().unwrap();
    let image = EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        data: vec![0; 16],
        rodata_size: 8,
        symbols: vec![
            EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            },
            EncodedSymbol {
                name: "_mfb_str_0".to_string(),
                section: EncodedSection::Data,
                offset: 0,
            },
        ],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: "_main".to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
        debug_symbols: true,
    };
    let path = crate::os::macos::write_linked_executable(dir.path(), "probe", &image).unwrap();
    let image = read(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(summary(&image), [("_main".to_string(), true)]);
    assert_eq!(
        image
            .data
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["_mfb_str_0"]
    );
}

#[test]
fn a_file_that_does_not_fit_its_headers_is_refused() {
    let bytes = linked(&["_main"], &["_exit"], true);
    assert_eq!(read(b"not a program"), None);
    assert_eq!(read(&bytes[..40]), None);
    // Every truncation either reads the same table or is refused; none panic.
    for length in (0..bytes.len()).step_by(97) {
        let _ = read(&bytes[..length]);
    }
    let mut corrupt = bytes.clone();
    corrupt[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(read(&corrupt), None);
}
