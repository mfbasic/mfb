//! The native-library locator's edges (`codegen/link/locator/link_locator.rs`): a
//! binding entry that declares no locator at all, a library listed twice in the
//! link set, and a lookup of a library the build never resolved.

use crate::binary_repr::{NativeLibraryEntry, NativeLibraryLocator, NativeLibraryTable};
use crate::codegen::link::locator::{resolve, LibraryTables, LinkLibraries, LinkTarget};
use crate::manifest::libraries::{LibType, Libc};

fn linux_x86_64() -> LinkTarget {
    LinkTarget {
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        libc: Some(Libc::Glibc),
    }
}

fn sqlite(locators: Vec<NativeLibraryLocator>) -> NativeLibraryTable {
    NativeLibraryTable {
        entries: vec![NativeLibraryEntry {
            logical: "sqlite3".to_string(),
            locators,
        }],
    }
}

/// An entry with no locators matches no target, and the refusal says the binding
/// declared nothing rather than printing an empty list.
#[test]
fn an_entry_with_no_locators_is_refused_as_declaring_nothing() {
    let err =
        resolve(&sqlite(Vec::new()), "sqlite3", &linux_x86_64()).expect_err("no locator can match");
    assert_eq!(err.rule(), "NATIVE_LIBRARY_NO_MATCH");
    let message = err.message();
    assert!(
        message.contains("The binding declares: nothing."),
        "the refusal names what was declared: {message}"
    );
}

/// A library named twice in the link set is resolved once, and asking for one the
/// build never resolved is an error naming it, not a default soname.
#[test]
fn a_library_is_resolved_once_and_an_unresolved_one_is_refused() {
    let system = NativeLibraryLocator {
        os: "linux".to_string(),
        arch: None,
        libc: None,
        lib_type: LibType::System,
        source: "libsqlite3.so.0".to_string(),
        hash: None,
    };
    let tables =
        LibraryTables::collect(&[], "app", sqlite(vec![system])).expect("no package to read");
    let linked = ["sqlite3".to_string(), "sqlite3".to_string()];
    let libraries =
        LinkLibraries::resolve_all(&tables, &linked, &linux_x86_64()).expect("resolves");
    let resolved = libraries.get("sqlite3").expect("resolved above");
    assert_eq!(resolved.dlopen_name, "libsqlite3.so.0");
    assert_eq!(resolved.declaring_unit, "app");
    let err = libraries.get("zlib").expect_err("never resolved");
    assert_eq!(
        err,
        "native library `zlib` was not resolved before code emission"
    );
}
