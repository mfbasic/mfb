// ---------------------------------------------------------------------------
// mod.rs — error-formatting closures on the public read/write entry points.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

#[test]
fn read_package_ir_with_identity_reports_bad_container() {
    // A file that is not a valid .mfp container trips the payload error map.
    let path = temp_mfp(b"not an mfp file at all, definitely too short");
    assert!(read_package_ir_with_identity(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_package_ir_with_identity_reports_bad_inner_payload() {
    // Valid container header wrapping garbage MFPC bytes.
    let path = temp_mfp(&wrap_mfp(b"MFPCnope", "pkg", "pkg", "1"));
    assert!(read_package_ir_with_identity(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_package_ir_with_identity_reports_identity_mismatch() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    // Header claims a name the manifest does not carry.
    let path = temp_mfp(&wrap_mfp(&inner, "WRONG", "WRONG", "1.0.0"));
    assert!(read_package_ir_with_identity(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_binary_repr_hex_reports_unwritable_directory() {
    // A directory that does not exist makes fs::write fail.
    let dir = std::path::Path::new("/nonexistent-dir-xyz/deeper");
    let result = write_binary_repr_hex(dir, &rich_project(), "1.0.0");
    assert!(result.is_err());
}

#[test]
fn read_package_helpers_report_inner_decode_errors() {
    // A container that parses at the container layer but whose inner MFPC is
    // truncated trips each public helper's error-formatting closure.
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    // Corrupt the inner MFPC major version so read_binary_repr_package fails.
    let mut broken = inner.clone();
    broken[4] = 0xFF;
    broken[5] = 0xFF;
    let path = temp_mfp(&wrap_mfp(&broken, "richpkg", "richpkg", "1.0.0"));
    assert!(read_package_exports(&path).is_err());
    assert!(read_package_info(&path).is_err());
    assert!(read_package_type_exports(&path).is_err());
    assert!(read_package_resources(&path).is_err());
    assert!(read_package_docs(&path).is_err());
    let _ = std::fs::remove_file(&path);
}
