//! Symbol-versioned imports across more than one library and version.
//!
//! `versioned_exit_image` has one import, so its `.gnu.version_r` has one library
//! with one version. A real program imports from `libc` and `libm` at several
//! glibc versions, and the loader resolves each import through the Verneed record
//! of its own library: a version string shared between libraries must be stored
//! once, two versions of one library must share its record, and every record but
//! the last must link to the next.

use super::*;

fn import(library: &str, symbol: &str, version: &str) -> EncodedImport {
    EncodedImport {
        library: library.to_string(),
        symbol: symbol.to_string(),
        kind: ImportKind::Function,
        version: Some(version.to_string()),
    }
}

fn image() -> EncodedImage {
    EncodedImage {
        imports: vec![
            import("libc.so.6", "_exit", "GLIBC_2.17"),
            import("libc.so.6", "_abort", "GLIBC_2.2.5"),
            import("libm.so.6", "sin", "GLIBC_2.17"),
        ],
        ..versioned_exit_image()
    }
}

fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Both versions appear in `.dynstr`, the one two libraries share only once, and
/// both libraries are named; the version-needs dynamic tags count two libraries.
#[test]
fn a_version_two_libraries_share_is_stored_once_and_both_libraries_get_a_record() {
    let image = image();
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(
        dir.path(),
        "multiver",
        "aarch64",
        LinuxFlavor::Glibc,
        &image,
    )
    .expect("link");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(occurrences(&bytes, b"\0GLIBC_2.17\0"), 1, "shared, so once");
    assert_eq!(occurrences(&bytes, b"\0GLIBC_2.2.5\0"), 1);
    for library in ["libc.so.6", "libm.so.6"] {
        assert!(
            occurrences(&bytes, format!("\0{library}\0").as_bytes()) >= 1,
            "{library} is a needed library"
        );
    }
}

/// The prefix computation that fixes each stub's GOT offset counts a shared
/// version string once, exactly as the payload builder appends it: counting it
/// per import would shift every stub's GOT slot.
#[test]
fn the_dynamic_prefix_counts_a_shared_version_string_once() {
    let shared = image();
    let mut distinct = image();
    distinct.imports[2].version = Some("GLIBC_2.27".to_string());
    // A distinct third version adds its string to `.dynstr`; the shared one added
    // nothing, so the prefix is strictly smaller.
    let grown =
        super::elf::dynamic_prefix_size(&distinct) - super::elf::dynamic_prefix_size(&shared);
    assert!(grown > 0, "a version not shared must be counted");
}

/// A RIP-relative displacement is a signed 32-bit field: one past either end is
/// refused with the distance, rather than wrapping to a different address.
#[test]
fn a_rip_relative_displacement_beyond_two_gib_is_refused() {
    assert_eq!(super::rel32(0x1000, 0x1000 - 4 + 0x10), Ok(-0x10));
    let beyond = (i32::MAX as u64) + 1 + 4;
    let error = super::rel32(beyond, 0).expect_err("one past the reach");
    assert!(
        error.contains("exceeds the \u{00b1}2 GiB reach of rel32"),
        "{error}"
    );
    assert!(super::rel32(0, beyond).is_err(), "and the same backwards");
}
