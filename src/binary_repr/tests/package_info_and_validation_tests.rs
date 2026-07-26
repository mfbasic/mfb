// ---------------------------------------------------------------------------
// builder.rs + reader.rs — package_info over imports/docs; ABI validation errs.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;
use crate::ir::IrSourceLoc;

fn dep_mfp() -> std::path::PathBuf {
    let mut dep = empty_project("dep");
    dep.functions = vec![fn_named("helper", "export", "function", "Integer")];
    let inner = encode_project(
        &dep,
        &BinaryReprMetadata::new("dep".to_string(), "1.0.0".to_string()),
    );
    temp_mfp(&wrap_mfp(&inner, "dep", "dep", "1.0.0"))
}

/// A consumer package importing `dep`, calling `dep.helper`, with docs.
fn consumer_with_import_and_docs() -> (IrProject, BinaryReprMetadata) {
    let mut consumer = empty_project("app");
    let mut main = fn_named("run", "export", "function", "Integer");
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
    consumer.docs = crate::ir::ProjectDocs {
        package: Some(crate::ir::IrPackageDoc {
            name: "app".to_string(),
            desc: vec![(0, "An app.".to_string())],
            deprecated: None,
        }),
        decls: vec![crate::ir::IrDocDecl {
            kind: crate::ir::IrDocKind::Func,
            name: "run".to_string(),
            signature: "EXPORT FUNC run() AS Integer".to_string(),
            group: String::new(),
            desc: vec![(0, "Runs.".to_string())],
            args: vec![],
            props: vec![],
            ret: "the answer".to_string(),
            errors: vec![],
            example: String::new(),
            internal: false,
            deprecated: None,
        }],
    };
    let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
    metadata.dependencies = vec![BinaryReprDependency {
        name: "dep".to_string(),
        ident: String::new(),
        version: "1.0.0".to_string(),
        pin: true,
        flags: 0,
    }];
    (consumer, metadata)
}

fn write_consumer() -> (std::path::PathBuf, std::path::PathBuf) {
    let dep = dep_mfp();
    let (consumer, metadata) = consumer_with_import_and_docs();
    let inner = build_package_binary_repr_bytes(&consumer, &metadata, std::slice::from_ref(&dep))
        .expect("build");
    let path = temp_mfp(&wrap_mfp(&inner, "app", "app", "1.0.0"));
    (path, dep)
}

#[test]
fn package_info_reports_imports_and_used_symbols() {
    let (path, dep) = write_consumer();
    let info = read_package_info(&path).expect("info");
    assert_eq!(info.import_count, 1);
    let import = &info.imports[0];
    assert_eq!(import.package_name, "dep");
    assert!(import.pin);
    // The consumer references `dep.helper`, recorded as a used symbol.
    assert!(import.used_symbols.iter().any(|s| s.name == "helper"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&dep);
}

#[test]
fn package_docs_round_trip_through_container() {
    let (path, dep) = write_consumer();
    let docs = read_package_docs(&path).expect("docs");
    assert!(!docs.is_empty());
    assert_eq!(docs.package.as_ref().unwrap().name, "app");
    assert!(docs.decls.iter().any(|d| d.name == "run"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&dep);
}

#[test]
fn read_package_binary_repr_decodes_import_and_doc_sections() {
    let (path, dep) = write_consumer();
    let package = read_package_binary_repr(&path).expect("decode");
    assert_eq!(package.project.imports.entries.len(), 1);
    assert!(!package.project.docs.is_empty());
    assert!(!package.project.abi.dep_edges.is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&dep);
}

#[test]
fn validate_abi_index_rejects_sig_hash_mismatch() {
    // Decode a valid package, then corrupt an ABI export sig hash and
    // re-validate: the sig-hash check must fail.
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let mut abi = package.project.abi.clone();
    if let Some(export) = abi.exports.first_mut() {
        export.sig_hash[0] ^= 0xFF;
    }
    let err = validate_abi_index(
        &abi,
        &package.exports,
        &package.project.imports,
        &package.project.strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    );
    assert!(err.is_err());
}

#[test]
fn validate_abi_index_rejects_missing_export_entry() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let mut abi = package.project.abi.clone();
    abi.exports.clear(); // now no ABI entry backs the EXPORT_TABLE
    let err = validate_abi_index(
        &abi,
        &package.exports,
        &package.project.imports,
        &package.project.strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    );
    assert!(err.is_err());
}

#[test]
fn validate_manifest_counts_rejects_a_manifest_that_lies_about_its_tables() {
    // bug-282 B4: the manifest repeats the dependency and export counts, and
    // both were decoded into `_`-prefixed locals and dropped -- so a manifest
    // could claim counts its own tables contradicted and no reader noticed.
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let manifest = &package.project.manifest;
    let imports = &package.project.imports;
    assert!(!package.exports.is_empty(), "fixture must export something");

    // The package as written agrees with itself.
    validate_manifest_counts(manifest, imports, &package.exports)
        .expect("a well-formed package agrees with its own manifest");

    // Overstating the export count is rejected...
    let mut lying = BinaryReprManifest {
        export_count: manifest.export_count + 1,
        ..*manifest
    };
    let err = validate_manifest_counts(&lying, imports, &package.exports)
        .expect_err("an overstated export count must be rejected");
    assert!(
        err.contains("manifest claims") && err.contains("exports"),
        "unexpected error: {err}"
    );

    // ...as is disagreeing about dependencies.
    lying = BinaryReprManifest {
        dependency_count: manifest.dependency_count + 1,
        ..*manifest
    };
    let err = validate_manifest_counts(&lying, imports, &package.exports)
        .expect_err("an overstated dependency count must be rejected");
    assert!(
        err.contains("manifest claims") && err.contains("dependencies"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_abi_index_rejects_callable_export_with_no_export_table_row() {
    // bug-282 B1: the callable verification loop is driven by EXPORT_TABLE, so
    // an ABI_INDEX Func/Sub entry naming no EXPORT_TABLE row was never reached
    // and its sigHash never recomputed -- it was accepted verbatim, then flowed
    // into `pkg info`, `repo check-abi` and the registry `abi_index`, where it
    // could satisfy an importer's used-symbol pin for a function that does not
    // exist. This is the callable-side mirror of the type asymmetry bug-21
    // closed.
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let mut abi = package.project.abi.clone();
    let mut strings = package.project.strings.clone();
    // A callable entry for a name no EXPORT_TABLE row carries, with a sig hash
    // that was never derived from anything.
    let ghost = strings.intern("ghostFunction");
    abi.exports.push(AbiExport {
        name: ghost,
        kind: BinaryReprExportKind::Func,
        sig_hash: [0x11; ABI_HASH_LEN],
    });
    let err = validate_abi_index(
        &abi,
        &package.exports,
        &package.project.imports,
        &strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    )
    .expect_err("an unbacked callable ABI entry must be rejected");
    assert!(
        err.contains("ghostFunction") && err.contains("export table"),
        "unexpected error: {err}"
    );
}

#[test]
fn abi_export_for_decoded_finds_matching_entry() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let package = read_binary_repr_package(&inner).expect("decode");
    let export = &package.exports[0];
    assert!(abi_export_for_decoded(&package.project.abi, export).is_some());
}
