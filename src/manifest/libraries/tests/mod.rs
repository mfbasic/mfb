use super::*;

fn manifest(json: &str) -> HashMap<String, JsonValue> {
    let value: JsonValue = json.parse().expect("test manifest parses");
    value
        .get::<HashMap<String, JsonValue>>()
        .cloned()
        .expect("test manifest is an object")
}

#[test]
fn absent_section_yields_empty_map() {
    let manifest = manifest(r#"{ "name": "demo" }"#);
    assert!(project_libraries(&manifest).is_empty());
}

#[test]
fn parses_multi_entry_section_in_key_order() {
    let manifest = manifest(
        r#"{
                "libraries": {
                    "zlib": [
                        { "os": "linux", "type": "system", "source": "libz.so.1" }
                    ],
                    "sqlite3": [
                        { "os": "macos", "type": "system", "source": "libsqlite3.dylib" },
                        { "os": "linux", "type": "system", "source": "libsqlite3.so.0" },
                        {
                            "os": "linux",
                            "arch": "riscv64",
                            "libc": "musl",
                            "source": "libsqlite3-riscv64-musl.so"
                        }
                    ]
                }
            }"#,
    );

    let libraries = project_libraries(&manifest);
    // BTreeMap: deterministic key order regardless of JSON order.
    assert_eq!(
        libraries.keys().collect::<Vec<_>>(),
        vec!["sqlite3", "zlib"]
    );

    assert_eq!(
        libraries["sqlite3"],
        vec![
            LibraryLocator {
                os: "macos".to_string(),
                arch: None,
                libc: None,
                lib_type: LibType::System,
                source: "libsqlite3.dylib".to_string(),
            },
            LibraryLocator {
                os: "linux".to_string(),
                arch: None,
                libc: None,
                lib_type: LibType::System,
                source: "libsqlite3.so.0".to_string(),
            },
            LibraryLocator {
                os: "linux".to_string(),
                arch: Some("riscv64".to_string()),
                libc: Some(Libc::Musl),
                // §3.1: absent `type` defaults to vendor.
                lib_type: LibType::Vendor,
                source: "libsqlite3-riscv64-musl.so".to_string(),
            },
        ]
    );
    assert_eq!(libraries["zlib"].len(), 1);
}

#[test]
fn absent_type_defaults_to_vendor() {
    let manifest = manifest(
        r#"{
                "libraries": {
                    "foo": [
                        { "os": "linux", "arch": "x86_64", "libc": "glibc", "source": "libfoo.so" }
                    ]
                }
            }"#,
    );
    let libraries = project_libraries(&manifest);
    assert_eq!(libraries["foo"][0].lib_type, LibType::Vendor);
}

#[test]
fn omitted_arch_and_libc_are_wildcards() {
    let manifest = manifest(
        r#"{
                "libraries": {
                    "foo": [ { "os": "linux", "type": "system", "source": "libfoo.so.1" } ]
                }
            }"#,
    );
    let libraries = project_libraries(&manifest);
    assert_eq!(libraries["foo"][0].arch, None);
    assert_eq!(libraries["foo"][0].libc, None);
}

#[test]
fn both_libc_tokens_parse() {
    let manifest = manifest(
        r#"{
                "libraries": {
                    "foo": [
                        { "os": "linux", "arch": "x86_64", "libc": "glibc", "source": "g.so" },
                        { "os": "linux", "arch": "x86_64", "libc": "musl", "source": "m.so" }
                    ]
                }
            }"#,
    );
    let libraries = project_libraries(&manifest);
    assert_eq!(libraries["foo"][0].libc, Some(Libc::Glibc));
    assert_eq!(libraries["foo"][1].libc, Some(Libc::Musl));
}

// ---- coverage matrix + table assembly (plan-46-B §4.2/§4.3) ----

#[test]
fn the_supported_matrix_is_the_registry_crossed_with_libc() {
    let slots = supported_target_slots();
    // macOS and Windows have no libc axis; every Linux backend contributes
    // both flavors.
    assert!(slots
        .iter()
        .any(|s| s.os == "macos" && s.arch == "aarch64" && s.libc.is_none()));
    // plan-47-B: windows-x86_64 is registered (non-executable), so it appears
    // as a single libc-less slot.
    assert!(slots
        .iter()
        .any(|s| s.os == "windows" && s.arch == "x86_64" && s.libc.is_none()));
    for arch in ["aarch64", "x86_64", "riscv64"] {
        for libc in [Libc::Glibc, Libc::Musl] {
            assert!(
                slots
                    .iter()
                    .any(|s| s.os == "linux" && s.arch == arch && s.libc == Some(libc)),
                "missing slot linux/{arch}/{}",
                libc.as_str()
            );
        }
    }
    assert!(
        slots.iter().all(|s| s.os != "macos" || s.libc.is_none()),
        "macOS slots must carry no libc"
    );
    // Five registered backends: macos-aarch64 + three Linux arches × 2 libc +
    // windows-x86_64 (plan-47-B, libc-less).
    assert_eq!(slots.len(), 8, "the matrix is 8 slots (7 + windows-x86_64)");
}

/// The regression the old "libc defaults to glibc" semantics would have caused:
/// one wildcard system entry must cover **all six** Linux slots.
#[test]
fn one_wildcard_system_entry_covers_every_linux_slot() {
    let locator = LibraryLocator {
        os: "linux".to_string(),
        arch: None,
        libc: None,
        lib_type: LibType::System,
        source: "libsqlite3.so.0".to_string(),
    };
    let linux: Vec<TargetSlot> = supported_target_slots()
        .into_iter()
        .filter(|slot| slot.os == "linux")
        .collect();
    assert_eq!(linux.len(), 6);
    for slot in &linux {
        assert!(
            locator_covers(&locator, slot),
            "wildcard system entry must cover {slot}"
        );
    }
    // ...and nothing on macOS.
    assert!(supported_target_slots()
        .iter()
        .filter(|slot| slot.os == "macos")
        .all(|slot| !locator_covers(&locator, slot)));
}

#[test]
fn a_concrete_locator_covers_exactly_its_slot() {
    let locator = LibraryLocator {
        os: "linux".to_string(),
        arch: Some("riscv64".to_string()),
        libc: Some(Libc::Musl),
        lib_type: LibType::Vendor,
        source: "libx-riscv64-musl.so".to_string(),
    };
    let covered: Vec<String> = supported_target_slots()
        .iter()
        .filter(|slot| locator_covers(&locator, slot))
        .map(|slot| slot.to_string())
        .collect();
    assert_eq!(covered, vec!["linux/riscv64/musl"]);
}

#[test]
fn a_wildcard_arch_locator_pinned_to_one_libc_covers_three_slots() {
    let locator = LibraryLocator {
        os: "linux".to_string(),
        arch: None,
        libc: Some(Libc::Musl),
        lib_type: LibType::System,
        source: "libx.so".to_string(),
    };
    let covered = supported_target_slots()
        .iter()
        .filter(|slot| locator_covers(&locator, slot))
        .count();
    assert_eq!(covered, 3, "one per Linux arch, musl only");
}

#[test]
fn explicit_system_type_parses() {
    let manifest = manifest(
        r#"{
                "libraries": {
                    "foo": [ { "os": "macos", "type": "system", "source": "libfoo.dylib" } ]
                }
            }"#,
    );
    let libraries = project_libraries(&manifest);
    assert_eq!(libraries["foo"][0].lib_type, LibType::System);
}

// ---- table assembly + the four checks (plan-46-B §4.3) ----

/// A manifest whose `sqlite3` entry covers macOS + all of Linux.
const FULL_COVERAGE: &str = r#"{
        "libraries": {
            "sqlite3": [
                { "os": "macos", "type": "system", "source": "libsqlite3.dylib" },
                { "os": "linux", "type": "system", "source": "libsqlite3.so.0" },
                { "os": "windows", "type": "system", "source": "sqlite3.dll" }
            ]
        }
    }"#;

fn build(
    json: &str,
    linked: &[&str],
    root: &Path,
) -> (NativeLibraryTable, Vec<NativeLibraryFinding>) {
    let linked: Vec<String> = linked.iter().map(|s| s.to_string()).collect();
    build_native_library_table(&manifest(json), &linked, root)
}

#[test]
fn a_fully_covering_manifest_builds_a_table_with_no_findings() {
    let dir = tempfile::tempdir().unwrap();
    let (table, findings) = build(FULL_COVERAGE, &["sqlite3"], dir.path());
    assert!(findings.is_empty(), "unexpected findings: {findings:#?}");
    assert_eq!(table.entries.len(), 1);
    assert_eq!(table.entries[0].logical, "sqlite3");
    // macos + linux + windows locators (plan-47-B added the windows slot).
    assert_eq!(table.entries[0].locators.len(), 3);
    // Every locator is `system`, so none carries a hash.
    assert!(table.entries[0].locators.iter().all(|l| l.hash.is_none()));
}

#[test]
fn a_link_with_no_libraries_entry_is_a_missing_error() {
    let dir = tempfile::tempdir().unwrap();
    let (table, findings) = build(FULL_COVERAGE, &["zlib"], dir.path());
    let missing: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "NATIVE_LIBRARY_MISSING")
        .collect();
    assert_eq!(missing.len(), 1, "findings: {findings:#?}");
    assert!(
        missing[0].message.contains("`LINK \"zlib\"`"),
        "message: {}",
        missing[0].message
    );
    // A library with no entry contributes nothing to the table.
    assert!(table.locators("zlib").is_none());
}

#[test]
fn an_unused_libraries_entry_warns_and_is_not_encoded() {
    let dir = tempfile::tempdir().unwrap();
    // `sqlite3` is declared but nothing LINKs it.
    let (table, findings) = build(FULL_COVERAGE, &[], dir.path());
    let unused: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "NATIVE_LIBRARY_UNUSED")
        .collect();
    assert_eq!(unused.len(), 1, "findings: {findings:#?}");
    assert!(unused[0].message.contains("sqlite3"));
    assert!(
        table.is_empty(),
        "an unused entry must not be encoded: {table:#?}"
    );
}

#[test]
fn a_macos_only_manifest_warns_once_per_uncovered_linux_slot() {
    let dir = tempfile::tempdir().unwrap();
    let (_table, findings) = build(
        r#"{ "libraries": { "sqlite3": [
                { "os": "macos", "type": "system", "source": "libsqlite3.dylib" }
            ] } }"#,
        &["sqlite3"],
        dir.path(),
    );
    let uncovered: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "NATIVE_LIBRARY_TARGET_UNCOVERED")
        .collect();
    // Six Linux slots + windows-x86_64 (plan-47-B): the macOS slot is the only
    // one this manifest covers.
    assert_eq!(
        uncovered.len(),
        7,
        "the six Linux slots + windows-x86_64: {findings:#?}"
    );
    assert!(uncovered
        .iter()
        .any(|f| f.message.contains("linux/riscv64/musl")));
    assert!(uncovered
        .iter()
        .any(|f| f.message.contains("windows/x86_64")));
    assert!(uncovered.iter().all(|f| !f.message.contains("macos")));
}

#[test]
fn a_wildcard_linux_entry_emits_zero_uncovered_warnings() {
    // The plan-46-B §4.2 regression check: `libc: None` covers both flavors,
    // so one line covers all six Linux slots.
    let dir = tempfile::tempdir().unwrap();
    let (_table, findings) = build(FULL_COVERAGE, &["sqlite3"], dir.path());
    assert!(
        !findings
            .iter()
            .any(|f| f.rule == "NATIVE_LIBRARY_TARGET_UNCOVERED"),
        "wildcard entries must cover everything: {findings:#?}"
    );
}

#[test]
fn a_vendor_locator_hashes_its_file_from_the_vendor_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("vendor")).unwrap();
    std::fs::write(
        dir.path().join("vendor").join("libfixture.so"),
        b"fixture bytes",
    )
    .unwrap();

    let (table, findings) = build(
        r#"{ "libraries": { "demo": [
                { "os": "linux", "arch": "x86_64", "libc": "glibc", "type": "vendor",
                  "source": "libfixture.so" }
            ] } }"#,
        &["demo"],
        dir.path(),
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.rule == "NATIVE_LIBRARY_SOURCE_UNREADABLE"),
        "findings: {findings:#?}"
    );
    let locator = &table.locators("demo").expect("demo present")[0];
    // sha256("fixture bytes") — pinned, so a change to the hashing path is caught.
    let hash = locator.hash.expect("vendor locator carries a hash");
    assert_eq!(
        hash,
        sha256_file(&dir.path().join("vendor").join("libfixture.so")).unwrap()
    );
    assert_ne!(hash, [0u8; 32]);
}

#[test]
fn a_vendor_locator_with_no_file_is_an_unreadable_error_naming_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let (_table, findings) = build(
        r#"{ "libraries": { "demo": [
                { "os": "linux", "arch": "x86_64", "libc": "glibc", "type": "vendor",
                  "source": "libmissing.so" }
            ] } }"#,
        &["demo"],
        dir.path(),
    );
    let unreadable: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == "NATIVE_LIBRARY_SOURCE_UNREADABLE")
        .collect();
    assert_eq!(unreadable.len(), 1, "findings: {findings:#?}");
    // "put the file in vendor/" is the entire fix, so the message must name
    // the full expected path.
    let expected = dir.path().join("vendor").join("libmissing.so");
    assert!(
        unreadable[0]
            .message
            .contains(&expected.display().to_string()),
        "message must name the expected path: {}",
        unreadable[0].message
    );
}

#[test]
fn vendor_path_is_flat_under_the_project_root() {
    assert_eq!(
        vendor_path(Path::new("/proj"), "libfoo.so"),
        Path::new("/proj/vendor/libfoo.so")
    );
}

#[test]
fn the_table_is_sorted_by_logical_name_for_deterministic_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let (table, _) = build(
        r#"{ "libraries": {
                "zlib": [ { "os": "linux", "type": "system", "source": "libz.so.1" } ],
                "sqlite3": [ { "os": "linux", "type": "system", "source": "libsqlite3.so.0" } ]
            } }"#,
        // Declared in the opposite order to the sorted one.
        &["zlib", "sqlite3"],
        dir.path(),
    );
    let names: Vec<&str> = table.entries.iter().map(|e| e.logical.as_str()).collect();
    assert_eq!(names, vec!["sqlite3", "zlib"]);
}
