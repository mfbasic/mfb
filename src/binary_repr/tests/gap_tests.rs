// ---------------------------------------------------------------------------
// Coverage top-up: thread type names, enum/union payloads, cleanups, and the
// remaining ABI-validation / decode error branches.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;
use crate::ir::{IrEnumMember, IrField, IrType, IrVariant};

#[test]
fn type_id_parses_thread_source_names() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let data = types.type_id(&mut strings, "Thread OF Integer TO String");
    let with_res = types.type_id(&mut strings, "Thread OF Integer RES File TO String");
    let worker = types.type_id(&mut strings, "ThreadWorker OF Integer TO String");
    assert!(data >= FIRST_TABLE_TYPE_ID);
    assert_ne!(data, with_res);
    assert_ne!(worker, data);
    let names = type_entry_names(&types, &strings.values).expect("names");
    assert!(names.values().any(|n| n == "Thread OF Integer TO String"));
    assert!(names
        .values()
        .any(|n| n == "ThreadWorker OF Integer TO String"));
}

#[test]
fn source_type_payload_encodes_enum_members_with_ordinals() {
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let enum_type = IrType {
        kind: "enum".to_string(),
        visibility: "export".to_string(),
        name: "Color".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: vec![
            IrEnumMember {
                name: "Red".to_string(),
            },
            IrEnumMember {
                name: "Green".to_string(),
            },
        ],
        loc: loc(),
        file: String::new(),
    };
    let source_types = std::collections::HashMap::new();
    let payload = source_type_payload(&mut strings, &mut types, &source_types, &enum_type)
        .expect("enum payload");
    assert_eq!(checked_u32_at(&payload, 0).unwrap(), 2); // member count
                                                         // Second field is the first member's ordinal (0).
    assert_eq!(checked_u32_at(&payload, 8).unwrap(), 0);
}

#[test]
fn concrete_union_variants_flattens_included_unions() {
    let base = IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: "Base".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![IrVariant {
            name: "A".to_string(),
            fields: vec![],
            loc: loc(),
        }],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    let derived = IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: "Derived".to_string(),
        fields: vec![],
        includes: vec!["Base".to_string()],
        variants: vec![IrVariant {
            name: "B".to_string(),
            fields: vec![IrField {
                visibility: None,
                name: "v".to_string(),
                type_: "Integer".to_string(),
                loc: loc(),
            }],
            loc: loc(),
        }],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    let mut source_types = std::collections::HashMap::new();
    source_types.insert("Base", &base);
    source_types.insert("Derived", &derived);
    let variants = concrete_union_variants(&source_types, &derived).expect("flatten");
    // Base's A followed by Derived's B.
    let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["A", "B"]);
}

#[test]
fn union_with_includes_round_trips_variants_through_encode() {
    // Exercise the writer's union-include flattening end to end.
    let mut project = empty_project("uni");
    project.types = vec![
        IrType {
            kind: "union".to_string(),
            visibility: "public".to_string(),
            name: "Base".to_string(),
            fields: vec![],
            includes: vec![],
            variants: vec![IrVariant {
                name: "A".to_string(),
                fields: vec![],
                loc: loc(),
            }],
            members: vec![],
            loc: loc(),
            file: String::new(),
        },
        IrType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "Derived".to_string(),
            fields: vec![],
            includes: vec!["Base".to_string()],
            variants: vec![IrVariant {
                name: "B".to_string(),
                fields: vec![],
                loc: loc(),
            }],
            members: vec![],
            loc: loc(),
            file: String::new(),
        },
    ];
    let metadata = BinaryReprMetadata::new("uni".to_string(), "1".to_string());
    let bytes = build_binary_repr_bytes(&project, &metadata).expect("encode");
    let package = read_binary_repr_package(&bytes).expect("decode");
    let type_exports = package_type_exports(&package).expect("type exports");
    let derived = type_exports
        .iter()
        .find(|t| t.name == "Derived")
        .expect("Derived");
    // The exported union carries both the included and own variants.
    assert_eq!(derived.variants.len(), 2);
}

#[test]
fn encode_functions_emits_registers_and_cleanups() {
    // The writer never emits cleanups from lowering, so build a project and
    // splice a cleanup + register into a function before re-encoding.
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let mut lowered = lower_project(&project, &metadata).expect("lower");
    lowered.functions[0].registers.push(Register {
        type_id: TYPE_INTEGER,
        flags: 0,
    });
    lowered.functions[0].cleanups.push(Cleanup {
        id: 7,
        start_pc: 1,
        end_pc: 2,
        resource_register: 0,
        close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
        flags: CLEANUP_FLAG_RECORD_SECONDARY_CLOSE_FAILURE,
    });
    let bytes = lowered.encode();
    let package = read_binary_repr_package(&bytes).expect("decode");
    let info = package_info(&package).expect("info");
    assert_eq!(info.cleanup_count, 1);
    let cleanup = &info.cleanups[0];
    assert_eq!(cleanup.cleanup_id, 7);
    assert!(cleanup.records_secondary_close_failure);
}

#[test]
fn validate_abi_index_rejects_dep_request_and_symbol_mismatches() {
    // Build a consumer with a dependency edge, then corrupt the edge's
    // version and used symbols to trip both disagreement branches.
    let mut consumer = empty_project("app");
    consumer.functions = vec![fn_named("run", "export", "function", "Integer")];
    let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
    metadata.dependencies = vec![BinaryReprDependency {
        name: "dep".to_string(),
        ident: String::new(),
        version: "1.0.0".to_string(),
        pin: false,
        flags: 0,
    }];
    let lowered = lower_project(&consumer, &metadata).expect("lower");
    let bytes = lowered.encode();
    let package = read_binary_repr_package(&bytes).expect("decode");

    // Version-request disagreement.
    let mut abi = package.project.abi.clone();
    abi.dep_edges[0].version_request = abi.dep_edges[0].version_request.wrapping_add(1);
    assert!(validate_abi_index(
        &abi,
        &package.exports,
        &package.project.imports,
        &package.project.strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    )
    .is_err());

    // Used-symbol count disagreement.
    let mut abi2 = package.project.abi.clone();
    abi2.dep_edges[0].used_symbols.push(AbiUsedSymbol {
        name: 0,
        sig_hash: [0; ABI_HASH_LEN],
    });
    assert!(validate_abi_index(
        &abi2,
        &package.exports,
        &package.project.imports,
        &package.project.strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    )
    .is_err());
}

#[test]
fn validate_abi_index_rejects_edge_set_mismatch() {
    let mut consumer = empty_project("app");
    consumer.functions = vec![fn_named("run", "export", "function", "Integer")];
    let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
    metadata.dependencies = vec![BinaryReprDependency {
        name: "dep".to_string(),
        ident: String::new(),
        version: "1".to_string(),
        pin: false,
        flags: 0,
    }];
    let lowered = lower_project(&consumer, &metadata).expect("lower");
    let package = read_binary_repr_package(&lowered.encode()).expect("decode");
    let mut abi = package.project.abi.clone();
    abi.dep_edges.clear(); // edges no longer match the IMPORT_TABLE set
    assert!(validate_abi_index(
        &abi,
        &package.exports,
        &package.project.imports,
        &package.project.strings.values,
        &package.project.types,
        &package.project.constants,
        &package.project.functions,
    )
    .is_err());
}

#[test]
fn package_type_exports_errors_when_type_missing_from_table() {
    // An ABI export naming a type absent from the type table is rejected.
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let mut package =
        read_binary_repr_package(&build_binary_repr_bytes(&project, &metadata).unwrap())
            .expect("decode");
    // Drop the type entries so exported Point/Shape/Color can't be resolved.
    package.project.types.entries.clear();
    package.project.types.ids.clear();
    assert!(package_type_exports(&package).is_err());
}

#[test]
fn read_binary_repr_package_rejects_duplicate_section() {
    // Hand-forge an MFPC whose section table lists the same id twice.
    let mut sections = vec![Section::new(SECTION_STRING_POOL, {
        let mut b = Vec::new();
        put_u32(&mut b, 0);
        b
    })];
    sections.push(Section::new(SECTION_STRING_POOL, {
        let mut b = Vec::new();
        put_u32(&mut b, 0);
        b
    }));
    let bytes = encode_sections(&sections);
    match read_binary_repr_package(&bytes) {
        Ok(_) => panic!("expected duplicate-section error"),
        Err(err) => assert!(err.contains("duplicate"), "got: {err}"),
    }
}

#[test]
fn read_binary_repr_package_reports_missing_required_sections() {
    // A well-formed MFPC that carries only the string pool is missing the
    // type/const/function/manifest/import/abi/export sections.
    let bytes = encode_sections(&[Section::new(SECTION_STRING_POOL, {
        let mut b = Vec::new();
        put_u32(&mut b, 0);
        b
    })]);
    assert!(read_binary_repr_package(&bytes).is_err());
}
