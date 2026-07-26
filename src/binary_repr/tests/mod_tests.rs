// ---------------------------------------------------------------------------
// mod.rs — public entry points and full end-to-end round-trips.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

#[test]
fn build_and_read_package_exports_end_to_end() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
    let exports = read_package_exports(&path).expect("exports");
    assert_eq!(exports.len(), 2);
    let info = read_package_info(&path).expect("info");
    assert_eq!(info.manifest_name, "richpkg");
    let type_exports = read_package_type_exports(&path).expect("type exports");
    assert!(type_exports.iter().any(|t| t.name == "Point"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_package_ir_with_identity_round_trips_the_ir() {
    let project = rich_project();
    let inner = encode_project(
        &project,
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
    let (id, ir) = read_package_ir_with_identity(&path).expect("ir with identity");
    assert_eq!(id.len(), 16);
    // The decoded IR carries the same function names as the source project.
    let decoded_names: std::collections::HashSet<String> =
        ir.functions.iter().map(|f| f.name.clone()).collect();
    for f in &project.functions {
        assert!(decoded_names.contains(&f.name));
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_package_docs_returns_docs_or_empty() {
    // A project with no DOC blocks yields empty docs.
    let inner = encode_project(
        &empty_project("nodocs"),
        &BinaryReprMetadata::new("nodocs".to_string(), "1".to_string()),
    );
    let path = temp_mfp(&wrap_mfp(&inner, "nodocs", "nodocs", "1"));
    let docs = read_package_docs(&path).expect("docs");
    assert!(docs.is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_package_resources_reads_standard_and_native() {
    let mut project = empty_project("linkpkg");
    project.native_resources = vec![crate::ir::IrNativeResource {
        name: "Conn".to_string(),
        visibility: "export".to_string(),
        close_function: "lib.close".to_string(),
        sendable: false,
        close_may_fail: true,
    }];
    let inner = encode_project(
        &project,
        &BinaryReprMetadata::new("linkpkg".to_string(), "1".to_string()),
    );
    let path = temp_mfp(&wrap_mfp(&inner, "linkpkg", "linkpkg", "1"));
    let resources = read_package_resources(&path).expect("resources");
    assert!(resources.iter().any(|r| r.type_name == "Conn"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn public_read_helpers_error_on_missing_file() {
    let missing = std::path::Path::new("/nonexistent/does-not-exist.mfp");
    assert!(read_package_exports(missing).is_err());
    assert!(read_package_info(missing).is_err());
    assert!(read_package_docs(missing).is_err());
    assert!(read_package_resources(missing).is_err());
    assert!(read_package_ir_with_identity(missing).is_err());
}

#[test]
fn write_binary_repr_hex_writes_a_hex_file() {
    let dir = std::env::temp_dir().join(format!(
        "mfb-binrepr-hex-{}-{}",
        std::process::id(),
        "richpkg"
    ));
    let _ = std::fs::create_dir_all(&dir);
    let project = rich_project();
    let path = write_binary_repr_hex(&dir, &project, "1.0.0").expect("write hex");
    let contents = std::fs::read_to_string(&path).expect("read hex");
    // Hex dump uses uppercase two-digit bytes; the MFPC magic leads.
    assert!(contents.starts_with("4D 46 50 43") || contents.starts_with("4D46"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn binary_repr_metadata_new_defaults_are_empty() {
    let metadata = BinaryReprMetadata::new("pkg".to_string(), "1.0.0".to_string());
    assert_eq!(metadata.name, "pkg");
    assert_eq!(metadata.version, "1.0.0");
    assert!(metadata.ident.is_empty());
    assert!(metadata.dependencies.is_empty());
}
