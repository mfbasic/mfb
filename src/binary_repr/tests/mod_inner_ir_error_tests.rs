// ---------------------------------------------------------------------------
// mod.rs — post-decode inner-IR error on read_package_ir_with_identity.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

/// Rebuild the inner MFPC of `rich_project` with a corrupt BINARY_REPR
/// section: the container + all metadata sections stay valid (so the package
/// and its identity decode) but `decode_binary_repr` on the payload fails.
fn container_with_corrupt_binary_repr() -> Vec<u8> {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let proj = &package.project;
    let sections = vec![
        Section::new(SECTION_MANIFEST, proj.encode_manifest()),
        Section::new(SECTION_STRING_POOL, proj.strings.encode()),
        Section::new(SECTION_TYPE_TABLE, proj.types.encode()),
        Section::new(SECTION_CONST_POOL, proj.constants.encode()),
        Section::new(SECTION_IMPORT_TABLE, proj.imports.encode()),
        Section::new(SECTION_EXPORT_TABLE, proj.encode_exports()),
        Section::new(SECTION_GLOBAL_TABLE, proj.encode_globals()),
        Section::new(
            SECTION_FUNCTION_TABLE,
            proj.encode_functions(&vec![(0u64, 0u64); proj.functions.len()]),
        ),
        // Garbage payload: not a valid Binary Representation blob.
        Section::new(SECTION_BINARY_REPR, b"not-a-binary-repr".to_vec()),
        Section::new(SECTION_ABI_INDEX, proj.abi.encode()),
    ];
    encode_sections(&sections)
}

#[test]
fn read_package_ir_with_identity_reports_inner_ir_decode_failure() {
    let inner = container_with_corrupt_binary_repr();
    // The package + identity decode, but the IR payload does not.
    assert!(read_binary_repr_package(&inner).is_ok());
    let path = temp_mfp(&wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0"));
    let err = read_package_ir_with_identity(&path);
    assert!(err.is_err());
    let _ = std::fs::remove_file(&path);
}
