// ---------------------------------------------------------------------------
// writer.rs — cross-package lowering (external function metadata + import path).
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;
use crate::ir::IrSourceLoc;

fn dep_project() -> IrProject {
    let mut dep = empty_project("dep");
    dep.functions = vec![fn_named("helper", "export", "function", "Integer")];
    dep
}

fn write_dep_mfp() -> std::path::PathBuf {
    let inner = encode_project(
        &dep_project(),
        &BinaryReprMetadata::new("dep".to_string(), "1.0.0".to_string()),
    );
    temp_mfp(&wrap_mfp(&inner, "dep", "dep", "1.0.0"))
}

#[test]
fn lower_package_project_resolves_external_calls() {
    let dep_path = write_dep_mfp();
    // A consumer that calls `dep.helper`.
    let mut consumer = empty_project("app");
    let mut main = fn_named("main", "export", "function", "Integer");
    main.body = vec![IrOp::Return {
        value: Some(IrValue::Call {
            target: "dep.helper".to_string(),
            args: vec![],
            loc: IrSourceLoc::default(),
            type_: "Integer".to_string(),
        }),
        loc: IrSourceLoc::default(),
    }];
    consumer.functions = vec![main];
    let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
    metadata.dependencies = vec![BinaryReprDependency {
        name: "dep".to_string(),
        ident: String::new(),
        version: "1.0.0".to_string(),
        pin: false,
        flags: 0,
    }];
    let lowered = lower_package_project(&consumer, &metadata, std::slice::from_ref(&dep_path))
        .expect("lower package");
    // The import table records the used symbol `helper`.
    let used = &lowered.imports.entries[0].used_symbols;
    assert_eq!(used.len(), 1);
    assert_eq!(
        string_at(&lowered.strings.values, used[0].name).unwrap(),
        "helper"
    );
    let _ = std::fs::remove_file(&dep_path);
}

#[test]
fn external_function_metadata_assigns_ids_and_hashes() {
    let dep_path = write_dep_mfp();
    let package = read_package_binary_repr(&dep_path).expect("decode dep");
    let (ids, returns, hashes) =
        external_function_metadata(5, std::slice::from_ref(&package)).expect("metadata");
    assert!(ids.contains_key("dep.helper"));
    // Base id 5 + the function's own id 0.
    assert_eq!(ids["dep.helper"], 5);
    assert_eq!(returns["dep.helper"], "Integer");
    assert!(hashes.contains_key("dep.helper"));
    let _ = std::fs::remove_file(&dep_path);
}

#[test]
fn build_package_binary_repr_bytes_round_trips() {
    let dep_path = write_dep_mfp();
    let consumer = empty_project("app");
    let metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
    let bytes =
        build_package_binary_repr_bytes(&consumer, &metadata, std::slice::from_ref(&dep_path))
            .expect("build");
    assert!(read_binary_repr_package(&bytes).is_ok());
    let _ = std::fs::remove_file(&dep_path);
}

// --- Experimental: does a consumer that names a dep's type emit FOREIGN_TYPE_KIND? ---
fn dep_with_type_project() -> IrProject {
    use crate::ir::{IrField, IrType};
    let mut dep = empty_project("dep");
    dep.types = vec![IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: "Widget".to_string(),
        fields: vec![IrField {
            visibility: Some("export".to_string()),
            name: "id".to_string(),
            type_: "Integer".to_string(),
            loc: crate::ir::IrSourceLoc::default(),
        }],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: crate::ir::IrSourceLoc::default(),
        file: "src/main.mfb".to_string(),
    }];
    dep
}

fn dep_meta(name: &str) -> BinaryReprMetadata {
    BinaryReprMetadata::new(name.to_string(), "1.0.0".to_string())
}

fn dependency(name: &str) -> BinaryReprDependency {
    BinaryReprDependency {
        name: name.to_string(),
        ident: String::new(),
        version: "1.0.0".to_string(),
        pin: false,
        flags: 0,
    }
}

/// bug-390: a package that references a dependency's exported type in its own
/// public API re-exports it as a `FOREIGN_TYPE_KIND` entry, and a consumer resolves
/// that back to the owner's real definition from the sibling `.mfp`. This drives
/// `read_package_foreign_type_refs`, `read_package_type_exports_resolved`'s
/// foreign branches, `package_type_exports`'s foreign marker, and
/// `external_type_metadata`'s re-export arm across a three-package chain
/// (dep → app → top), all written into one directory so the resolver finds them.
#[test]
fn foreign_type_reexport_chain_resolves_through_siblings() {
    let dir = tempfile::tempdir().unwrap();

    // dep owns `Widget`.
    let dep_bytes = wrap_mfp(
        &encode_project(&dep_with_type_project(), &dep_meta("dep")),
        "dep",
        "dep",
        "1.0.0",
    );
    let dep_path = dir.path().join("dep.mfp");
    std::fs::write(&dep_path, &dep_bytes).unwrap();

    // app imports dep and re-exports `Widget` through an exported function.
    let mut app = empty_project("app");
    let mut make = fn_named("makeWidget", "export", "function", "Widget");
    make.returns = "Widget".to_string();
    app.functions = vec![make];
    let mut app_meta = dep_meta("app");
    app_meta.dependencies = vec![dependency("dep")];
    let app_inner = build_package_binary_repr_bytes(&app, &app_meta, std::slice::from_ref(&dep_path))
        .expect("build app");
    let app_path = dir.path().join("app.mfp");
    std::fs::write(&app_path, wrap_mfp(&app_inner, "app", "app", "1.0.0")).unwrap();

    // read_package_foreign_type_refs: the FOREIGN_TYPE_KIND entry body.
    let refs = read_package_foreign_type_refs(&app_path).expect("refs");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].name, "Widget");
    assert_eq!(refs[0].owner, "dep");

    // read_package_type_exports resolves the foreign marker against dep.mfp
    // (the owner-present branch fills fields from the owner's definition).
    let texp = read_package_type_exports(&app_path).expect("type exports");
    let widget = texp.iter().find(|t| t.name == "Widget").expect("Widget");
    assert_eq!(widget.foreign_owner.as_deref(), Some("dep"));
    assert_eq!(widget.fields.len(), 1, "fields resolved from the owner");
    assert_eq!(widget.fields[0].name, "id");

    // top imports app and references `Widget`; lowering it drives
    // external_type_metadata's re-export arm (owner carried through unchanged).
    let mut top = empty_project("top");
    let mut use_widget = fn_named("useWidget", "export", "function", "Widget");
    use_widget.returns = "Widget".to_string();
    top.functions = vec![use_widget];
    let mut top_meta = dep_meta("top");
    top_meta.dependencies = vec![dependency("app")];
    let top_inner = build_package_binary_repr_bytes(&top, &top_meta, std::slice::from_ref(&app_path))
        .expect("build top");
    let top_path = dir.path().join("top.mfp");
    std::fs::write(&top_path, wrap_mfp(&top_inner, "top", "top", "1.0.0")).unwrap();
    // top's own foreign ref carries dep as the ultimate owner, not app.
    let top_refs = read_package_foreign_type_refs(&top_path).expect("top refs");
    assert_eq!(top_refs.len(), 1);
    assert_eq!(top_refs[0].owner, "dep", "owner carries through the intermediary");
}
