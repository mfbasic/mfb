//! `NATIVE_LIBRARY_TABLE` (section id 10) codec — plan-46-B §4.1.
//!
//! The `.mfp` is an untrusted input on the consumer side and plan-46-C feeds a
//! locator's `source` straight into a `dlopen` C string and a filesystem path, so
//! the decode tests below matter as much as the round-trip: every one asserts a
//! structural error rather than a panic or a silently-accepted value.

use super::*;
use crate::manifest::libraries::{LibType, Libc};

fn system(os: &str, source: &str) -> NativeLibraryLocator {
    NativeLibraryLocator {
        os: os.to_string(),
        arch: None,
        libc: None,
        lib_type: LibType::System,
        source: source.to_string(),
        hash: None,
    }
}

fn populated() -> NativeLibraryTable {
    NativeLibraryTable {
        entries: vec![
            NativeLibraryEntry {
                logical: "imaging".to_string(),
                locators: vec![NativeLibraryLocator {
                    // A macOS vendor locator may omit `arch` — fat Mach-O.
                    os: "macos".to_string(),
                    arch: None,
                    libc: None,
                    lib_type: LibType::Vendor,
                    source: "libimaging.dylib".to_string(),
                    hash: Some([7u8; 32]),
                }],
            },
            NativeLibraryEntry {
                logical: "sqlite3".to_string(),
                locators: vec![
                    // Wildcard arch + libc: one entry covering all of Linux.
                    system("linux", "libsqlite3.so.0"),
                    system("macos", "libsqlite3.dylib"),
                    // Both libc values, concrete arch, vendor + hash.
                    NativeLibraryLocator {
                        os: "linux".to_string(),
                        arch: Some("riscv64".to_string()),
                        libc: Some(Libc::Musl),
                        lib_type: LibType::Vendor,
                        source: "libsqlite3-riscv64-musl.so".to_string(),
                        hash: Some([3u8; 32]),
                    },
                    NativeLibraryLocator {
                        os: "linux".to_string(),
                        arch: Some("x86_64".to_string()),
                        libc: Some(Libc::Glibc),
                        lib_type: LibType::Vendor,
                        source: "libsqlite3-x86_64-glibc.so".to_string(),
                        hash: Some([9u8; 32]),
                    },
                ],
            },
        ],
    }
}

/// Encode a table and decode it back through the same string pool.
fn round_trip(table: &NativeLibraryTable) -> Result<NativeLibraryTable, String> {
    let mut strings = StringPool::new();
    let bytes = encode_native_library_table(&mut strings, table);
    read_native_library_table(&bytes, &strings.values)
}

#[test]
fn populated_table_round_trips() {
    let table = populated();
    assert_eq!(round_trip(&table).expect("decodes"), table);
}

#[test]
fn empty_table_round_trips_and_is_empty() {
    let table = NativeLibraryTable::default();
    assert!(table.is_empty());
    assert_eq!(round_trip(&table).expect("decodes"), table);
}

#[test]
fn a_table_with_trailing_bytes_is_rejected() {
    // bug-282 B3: section 10 was added after audit-1 PKG-05 fixed the same
    // omission on the resource table, and reintroduced it -- garbage past the
    // last locator decoded silently.
    let mut strings = StringPool::new();
    let mut bytes = encode_native_library_table(&mut strings, &populated());
    bytes.push(0xAA);
    let err = read_native_library_table(&bytes, &strings.values)
        .expect_err("trailing garbage must be rejected");
    assert!(err.contains("trailing bytes"), "unexpected error: {err}");
}

#[test]
fn locators_keep_their_order_and_wildcards() {
    let decoded = round_trip(&populated()).expect("decodes");
    let sqlite3 = decoded.locators("sqlite3").expect("sqlite3 present");
    assert_eq!(sqlite3.len(), 4);
    assert_eq!(sqlite3[0].arch, None, "wildcard arch decodes as None");
    assert_eq!(sqlite3[0].libc, None, "wildcard libc decodes as None");
    assert_eq!(sqlite3[2].libc, Some(Libc::Musl));
    assert_eq!(sqlite3[3].libc, Some(Libc::Glibc));
    assert_eq!(decoded.locators("nope"), None);
}

#[test]
fn system_locators_carry_no_hash_and_vendor_locators_do() {
    let decoded = round_trip(&populated()).expect("decodes");
    for locator in decoded.locators("sqlite3").expect("present") {
        match locator.lib_type {
            LibType::System => assert!(locator.hash.is_none(), "system carries no hash"),
            LibType::Vendor => assert!(locator.hash.is_some(), "vendor carries a hash"),
        }
    }
}

/// Hand-build a one-locator section so the malformed cases below can corrupt a
/// single field. Returns (bytes, string pool).
fn one_locator(
    os: &str,
    arch: &str,
    libc: u8,
    lib_type: u8,
    source: &str,
) -> (Vec<u8>, Vec<String>) {
    let mut strings = StringPool::new();
    let logical = strings.intern("sqlite3");
    let os = strings.intern(os);
    let arch = strings.intern(arch);
    let source = strings.intern(source);
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1); // libraryCount
    put_u32(&mut bytes, logical);
    put_u32(&mut bytes, 1); // locatorCount
    put_u32(&mut bytes, os);
    put_u32(&mut bytes, arch);
    bytes.push(libc);
    bytes.push(lib_type);
    put_u32(&mut bytes, source);
    if lib_type == WIRE_LIB_TYPE_VENDOR {
        bytes.extend_from_slice(&[0u8; 32]);
    }
    (bytes, strings.values)
}

#[test]
fn out_of_range_libc_is_a_structural_error() {
    let (bytes, strings) = one_locator("linux", "", 9, WIRE_LIB_TYPE_SYSTEM, "libx.so");
    let error = read_native_library_table(&bytes, &strings).expect_err("must reject");
    assert!(error.contains("out-of-range libc"), "error: {error}");
}

#[test]
fn out_of_range_type_is_a_structural_error() {
    let (bytes, strings) = one_locator("linux", "", WIRE_LIBC_UNSPECIFIED, 7, "libx.so");
    let error = read_native_library_table(&bytes, &strings).expect_err("must reject");
    assert!(error.contains("out-of-range type"), "error: {error}");
}

#[test]
fn a_source_with_a_path_separator_is_rejected_on_decode() {
    // A hostile `.mfp` must not reach the `vendor/` path join or the dlopen
    // string: the decoder re-validates rather than trusting the producer.
    let (bytes, strings) = one_locator(
        "linux",
        "",
        WIRE_LIBC_UNSPECIFIED,
        WIRE_LIB_TYPE_SYSTEM,
        "../../etc/passwd",
    );
    let error = read_native_library_table(&bytes, &strings).expect_err("must reject");
    assert!(error.contains("not a bare filename"), "error: {error}");
}

#[test]
fn a_source_with_an_interior_nul_is_rejected_on_decode() {
    let (bytes, strings) = one_locator(
        "linux",
        "",
        WIRE_LIBC_UNSPECIFIED,
        WIRE_LIB_TYPE_SYSTEM,
        "libx.so\0evil",
    );
    let error = read_native_library_table(&bytes, &strings).expect_err("must reject");
    assert!(error.contains("not a bare filename"), "error: {error}");
}

#[test]
fn a_vendor_locator_with_a_truncated_hash_is_rejected() {
    let (mut bytes, strings) = one_locator(
        "linux",
        "",
        WIRE_LIBC_UNSPECIFIED,
        WIRE_LIB_TYPE_VENDOR,
        "libx.so",
    );
    bytes.truncate(bytes.len() - 4);
    let error = read_native_library_table(&bytes, &strings).expect_err("must reject");
    assert!(error.contains("truncated"), "error: {error}");
}

#[test]
fn an_unknown_string_id_is_rejected_rather_than_panicking() {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1);
    put_u32(&mut bytes, 99); // logical name id past the end of the pool
    put_u32(&mut bytes, 0);
    let error =
        read_native_library_table(&bytes, &["sqlite3".to_string()]).expect_err("must reject");
    assert!(error.contains("unknown string id"), "error: {error}");
}

#[test]
fn a_truncated_table_is_rejected_rather_than_panicking() {
    let error = read_native_library_table(&[0, 0], &[]).expect_err("must reject");
    assert!(!error.is_empty());
}
