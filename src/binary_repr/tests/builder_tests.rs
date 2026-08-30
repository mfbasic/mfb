// ---------------------------------------------------------------------------
// builder.rs — package_exports / package_info / package_type_exports / resources.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

fn decoded_package() -> PackageBinaryRepr {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    read_binary_repr_package(&inner).expect("decode package")
}

#[test]
fn package_exports_lists_callables_with_signatures() {
    let package = decoded_package();
    let exports = package_exports(&package).expect("exports");
    let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"main"));
    assert!(names.contains(&"doThing"));
    let main = exports.iter().find(|e| e.name == "main").unwrap();
    assert_eq!(main.return_type.name(), "Integer");
    assert_eq!(main.params.len(), 2);
    // The defaulted parameter carries has_default.
    assert!(main.params[1].has_default);
    assert!(!main.params[0].has_default);
}

#[test]
fn package_info_reports_counts_and_metadata() {
    let package = decoded_package();
    let info = package_info(&package).expect("info");
    assert_eq!(info.manifest_name, "richpkg");
    assert_eq!(info.manifest_version, "1.0.0");
    assert_eq!(info.function_count, 3);
    assert_eq!(info.global_count, 3);
    // Two exported callables plus exported Point/Shape/Color type exports.
    assert!(info.export_count >= 2);
    assert_eq!(info.abi_format_version, ABI_FORMAT_VERSION);
    // Globals report visibility strings.
    let visibilities: Vec<&str> = info.globals.iter().map(|g| g.visibility.as_str()).collect();
    assert!(visibilities.contains(&"private"));
    assert!(visibilities.contains(&"public"));
    assert!(visibilities.contains(&"export"));
}

#[test]
fn package_type_exports_decodes_record_union_enum() {
    let package = decoded_package();
    let types = package_type_exports(&package).expect("type exports");
    let point = types.iter().find(|t| t.name == "Point").expect("Point");
    assert!(point.kind == BinaryReprExportKind::Type);
    assert_eq!(point.fields.len(), 2);
    let shape = types.iter().find(|t| t.name == "Shape").expect("Shape");
    assert!(shape.kind == BinaryReprExportKind::Union);
    assert_eq!(shape.variants.len(), 1);
    let color = types.iter().find(|t| t.name == "Color").expect("Color");
    assert!(color.kind == BinaryReprExportKind::Enum);
    assert_eq!(color.members, vec!["Red".to_string(), "Green".to_string()]);
}

#[test]
fn resolve_resource_close_name_maps_builtins_and_functions() {
    let package = decoded_package();
    assert_eq!(
        resolve_resource_close_name(&package, BUILTIN_FS_CLOSE_FUNCTION_ID).unwrap(),
        builtins::resource_close_function(&crate::types::ParameterType::named(
            crate::codegen::builtins::fs::FILE_TYPE_ID
        ))
        .map(str::to_string)
    );
    assert_eq!(
        resolve_resource_close_name(&package, BUILTIN_NET_CLOSE_FUNCTION_ID).unwrap(),
        builtins::resource_close_function(&crate::types::ParameterType::named(
            crate::codegen::builtins::net::SOCKET_TYPE
        ))
        .map(str::to_string)
    );
    // A function-id index resolves to that function's name.
    let named = resolve_resource_close_name(&package, 0).unwrap();
    assert!(named.is_some());
    // An out-of-range id resolves to None.
    assert!(resolve_resource_close_name(&package, u32::MAX - 5)
        .unwrap()
        .is_none());
}

#[test]
fn package_resource_exports_decodes_native_link_resource() {
    let mut project = empty_project("linkpkg");
    project.native_resources = vec![crate::ir::IrNativeResource {
        name: "Db".to_string(),
        visibility: "export".to_string(),
        close_function: "lib.close".to_string(),
        sendable: true,
        close_may_fail: true,
    }];
    let inner = encode_project(
        &project,
        &BinaryReprMetadata::new("linkpkg".to_string(), "1".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let resources = package_resource_exports(&package).expect("resources");
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].type_name, "Db");
    assert!(resources[0].native);
    assert!(resources[0].sendable);
    assert_eq!(resources[0].close_function.as_deref(), Some("lib.close"));
}

#[test]
fn package_exports_reports_a_missing_function() {
    let mut package = decoded_package();
    assert!(!package.exports.is_empty());
    // Point the first export at a function id past the end of the table.
    package.exports[0].function_id = 9999;
    let err = package_exports(&package).map(|_| ()).unwrap_err();
    assert!(err.contains("references missing function"), "{err}");
}
