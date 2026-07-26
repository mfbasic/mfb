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
