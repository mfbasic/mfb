// ---------------------------------------------------------------------------
// writer.rs — lowering an IrProject to the section model + helper parsers.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

#[test]
fn lower_project_populates_all_tables() {
    let project = rich_project();
    let metadata = BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string());
    let lowered = lower_project(&project, &metadata).expect("lower");
    assert_eq!(lowered.functions.len(), 3);
    assert_eq!(lowered.globals.len(), 3);
    // Point, Shape, Color plus any composite (List OF Integer) types.
    assert!(lowered.types.entries.len() >= 3);
    // Entry function flags: bit0 set, args bit set, Integer-return bit set.
    assert_eq!(lowered.entry_flags & 1, 1);
    assert_eq!(lowered.entry_flags & (1 << 1), 1 << 1);
    assert_eq!(lowered.entry_flags & (1 << 2), 1 << 2);
    assert_ne!(lowered.entry_function, u32::MAX);
    // Exactly the two exported callables (main, doThing) are exported.
    assert_eq!(lowered.export_count(), 2);
}

#[test]
fn lower_project_without_entry_uses_sentinel() {
    let mut project = empty_project("noentry");
    project.functions = vec![fn_named("f", "private", "function", "Integer")];
    let metadata = BinaryReprMetadata::new("noentry".to_string(), "1".to_string());
    let lowered = lower_project(&project, &metadata).expect("lower");
    assert_eq!(lowered.entry_function, u32::MAX);
    assert_eq!(lowered.entry_flags, 0);
    assert_eq!(lowered.export_count(), 0);
}

#[test]
fn lower_project_missing_entry_function_errors() {
    let mut project = empty_project("badentry");
    project.entry = Some(crate::ir::EntryPoint {
        name: "ghost".to_string(),
        returns: crate::types::ParameterType::parse("Nothing"),
        accepts_args: false,
    });
    let metadata = BinaryReprMetadata::new("badentry".to_string(), "1".to_string());
    assert!(lower_project(&project, &metadata).is_err());
}

#[test]
fn native_resources_add_type_and_resource_entry() {
    let mut project = empty_project("linkpkg");
    project.native_resources = vec![
        crate::ir::IrNativeResource {
            name: "Db".to_string(),
            visibility: "export".to_string(),
            close_function: "lib.close".to_string(),
            sendable: true,
            close_may_fail: true,
        },
        crate::ir::IrNativeResource {
            name: "Priv".to_string(),
            visibility: "private".to_string(),
            close_function: "lib.shut".to_string(),
            sendable: false,
            close_may_fail: false,
        },
    ];
    let metadata = BinaryReprMetadata::new("linkpkg".to_string(), "1".to_string());
    let lowered = lower_project(&project, &metadata).expect("lower");
    assert_eq!(lowered.resources.entries.len(), 2);
    // The exported native resource carries an ABI export kind.
    assert!(lowered
        .types
        .entries
        .iter()
        .any(|entry| entry.abi_export_kind.is_some()));
}

/// A wire function signature's parameter list splits at TOP-LEVEL commas only.
///
/// plan-106-E: this used to test `split_top_level_types` directly, a local
/// splitter the wire encoder called after stripping `FUNC(` itself. Both are
/// deleted — the rule lives in `ParameterType::parse` — so the same contract is
/// pinned through the entry point that survives.
#[test]
fn split_top_level_types_respects_nesting() {
    let empty = parse_function_type("FUNC() AS Nothing").expect("a no-parameter signature");
    assert!(empty.params.is_empty());
    let two = parse_function_type("FUNC(Integer, String) AS Nothing").expect("two parameters");
    assert_eq!(
        two.params,
        vec!["Integer".to_string(), "String".to_string()]
    );
    // A comma inside a nested FUNC(...) is not a top-level separator.
    let nested = parse_function_type("FUNC(FUNC(Integer, String) AS Boolean, Byte) AS Nothing")
        .expect("a higher-order signature");
    assert_eq!(
        nested.params,
        vec![
            "FUNC(Integer, String) AS Boolean".to_string(),
            "Byte".to_string()
        ]
    );
}

#[test]
fn parse_function_type_handles_isolated_and_plain() {
    let plain = parse_function_type("FUNC(Integer, String) AS Boolean").unwrap();
    assert!(!plain.isolated);
    assert_eq!(
        plain.params,
        vec!["Integer".to_string(), "String".to_string()]
    );
    assert_eq!(plain.returns, "Boolean");

    let iso = parse_function_type("ISOLATED FUNC() AS Nothing").unwrap();
    assert!(iso.isolated);
    assert!(iso.params.is_empty());
    assert_eq!(iso.returns, "Nothing");

    // Not a function type.
    assert!(parse_function_type("Integer").is_none());
    // Missing ") AS " terminator.
    assert!(parse_function_type("FUNC(Integer").is_none());
}

/// The parameter list and the return type split at the TOP-LEVEL `") AS "`.
///
/// plan-106-E: was a direct test of `split_function_type_rest`, the wire
/// encoder's own depth scanner, now deleted along with it. The contract it
/// protected — a nested `FUNC(...) AS T` parameter must not split at its INNER
/// `") AS "` (bug-175 F) — is pinned here through `parse_function_type`.
#[test]
fn split_function_type_rest_finds_top_level_terminator() {
    let simple = parse_function_type("FUNC(Integer) AS Boolean").expect("a simple signature");
    assert_eq!(simple.params, vec!["Integer".to_string()]);
    assert_eq!(simple.returns, "Boolean");
    // Nested parens are skipped until the top-level ") AS ".
    let nested =
        parse_function_type("FUNC(FUNC() AS Integer) AS Boolean").expect("a nested signature");
    assert_eq!(nested.params, vec!["FUNC() AS Integer".to_string()]);
    assert_eq!(nested.returns, "Boolean");
    // A return type that is itself a function keeps its whole spelling.
    let returns_func =
        parse_function_type("FUNC(Integer) AS FUNC(Integer) AS Integer").expect("a returned FUNC");
    assert_eq!(returns_func.returns, "FUNC(Integer) AS Integer");
    // Not a function type at all.
    assert!(parse_function_type("Integer").is_none());
}

#[test]
fn fixed_raw_from_decimal_covers_signs_fractions_and_rounding() {
    // Whole number.
    assert_eq!(fixed_raw_from_decimal("2").unwrap(), 2i64 << 32);
    // Negative.
    assert_eq!(fixed_raw_from_decimal("-2").unwrap(), -(2i64 << 32));
    // 0.5 == half of the scale.
    assert_eq!(fixed_raw_from_decimal("0.5").unwrap(), 1i64 << 31);
    // Leading-dot form.
    assert_eq!(fixed_raw_from_decimal(".5").unwrap(), 1i64 << 31);
    // Rounds up when the fractional remainder is >= half.
    let quarter = fixed_raw_from_decimal("0.25").unwrap();
    assert_eq!(quarter, 1i64 << 30);
}

#[test]
fn fixed_raw_from_decimal_rejects_malformed() {
    assert!(fixed_raw_from_decimal("").is_err());
    assert!(fixed_raw_from_decimal(".").is_err());
    assert!(fixed_raw_from_decimal("1.2x").is_err());
    assert!(fixed_raw_from_decimal("notanumber").is_err());
    // Out of i64 range after scaling.
    assert!(fixed_raw_from_decimal("99999999999999").is_err());
}

#[test]
fn ir_uses_resource_type_detects_file_param() {
    let mut project = empty_project("res");
    let mut f = fn_named("takesFile", "export", "sub", "Nothing");
    f.params = vec![crate::ir::IrParam {
        name: "h".to_string(),
        type_: crate::types::ParameterType::parse("fs.File"),
        default: None,
        loc: loc(),
    }];
    project.functions = vec![f];
    assert!(ir_uses_resource_type(&project));

    let plain = empty_project("plain");
    assert!(!ir_uses_resource_type(&plain));
}

#[test]
fn is_resource_type_name_matches_builtins() {
    assert!(is_resource_type_name("fs.File"));
    // plan-110-E: the stream handles are tcp's now; net has no resources.
    assert!(is_resource_type_name("tcp.Socket"));
    assert!(!is_resource_type_name("net.Socket"));
    assert!(!is_resource_type_name("Integer"));
}

#[test]
fn standard_resource_flags_marks_sendable_types() {
    let file = standard_resource_flags(crate::codegen::builtins::fs::FILE_TYPE_ID);
    assert!(file & RESOURCE_FLAG_SENDABLE != 0);
    let listener = standard_resource_flags(crate::codegen::builtins::tcp::LISTENER_TYPE_ID);
    // bug-464 made the Listener sendable; `process::Process` is the negative
    // exemplar now, so this still proves the bit tracks the registry rather than
    // being unconditionally set.
    assert!(listener & RESOURCE_FLAG_SENDABLE != 0);
    let process = standard_resource_flags(crate::codegen::builtins::process::PROCESS_TYPE_ID);
    assert!(process & RESOURCE_FLAG_SENDABLE == 0);
}

#[test]
fn source_type_payload_encodes_union_and_enum() {
    use crate::ir::{IrField, IrType, IrVariant};
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let union = IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: "U".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![IrVariant {
            name: "A".to_string(),
            fields: vec![IrField {
                visibility: None,
                name: "v".to_string(),
                type_: crate::types::ParameterType::parse("Integer"),
                loc: loc(),
            }],
            loc: loc(),
        }],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    let source_types = std::collections::HashMap::new();
    let payload = source_type_payload(&mut strings, &mut types, &source_types, &union)
        .expect("union payload");
    // First u32 is the variant count.
    assert_eq!(checked_u32_at(&payload, 0).unwrap(), 1);
}

#[test]
fn concrete_union_variants_rejects_unknown_include() {
    use crate::ir::IrType;
    let bad = IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: "U".to_string(),
        fields: vec![],
        includes: vec!["Missing".to_string()],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    let source_types = std::collections::HashMap::new();
    assert!(concrete_union_variants(&source_types, &bad).is_err());
}

#[test]
fn fixed_raw_from_decimal_covers_long_fractions_and_carries() {
    // Round-half-up carry: 0.3 leaves a nonzero remainder that rounds up.
    assert!(fixed_raw_from_decimal("0.3").is_ok());
    // A fraction that rounds up to a whole one (the `fractional_value == SCALE`
    // carry into the whole part).
    assert_eq!(fixed_raw_from_decimal("0.99999999999").unwrap(), 1i64 << 32);
    // 28 valid fractional digits then a non-digit past the 28-digit cap still errs.
    let bad_tail = format!("0.{}x", "1".repeat(28));
    assert!(fixed_raw_from_decimal(&bad_tail).is_err());
    // Many valid digits past the cap are accepted (they sit below one ULP).
    let long_ok = format!("0.{}", "1".repeat(40));
    assert!(fixed_raw_from_decimal(&long_ok).is_ok());
}

#[test]
fn source_type_payload_records_encode_field_visibilities() {
    use crate::ir::{IrField, IrType};
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let field = |vis: Option<&str>, name: &str| IrField {
        visibility: vis.map(str::to_string),
        name: name.to_string(),
        type_: crate::types::ParameterType::parse("Integer"),
        loc: loc(),
    };
    let record = IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: "R".to_string(),
        fields: vec![
            field(Some("private"), "a"),
            field(Some("package"), "b"),
            field(Some("export"), "c"),
            field(None, "d"),
        ],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    let source_types = std::collections::HashMap::new();
    let payload =
        source_type_payload(&mut strings, &mut types, &source_types, &record).expect("record");
    // First u32 is the field count.
    assert_eq!(checked_u32_at(&payload, 0).unwrap(), 4);
}

#[test]
fn source_type_payload_ignores_a_non_composite_kind() {
    use crate::ir::IrType;
    let mut strings = StringPool::new();
    let mut types = TypeTable::new();
    let alias = IrType {
        kind: "alias".to_string(),
        visibility: "export".to_string(),
        name: "A".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    let source_types = std::collections::HashMap::new();
    let payload =
        source_type_payload(&mut strings, &mut types, &source_types, &alias).expect("alias");
    assert!(payload.is_empty(), "an unknown kind encodes no payload");
}

#[test]
fn lower_project_encodes_the_doc_table_from_ir() {
    use crate::ir::{IrDocDecl, IrDocKind, IrPackageDoc, ProjectDocs};
    let decl = |kind, name: &str| IrDocDecl {
        kind,
        name: name.to_string(),
        signature: format!("{name}()"),
        group: String::new(),
        desc: vec![],
        args: vec![],
        props: vec![],
        ret: String::new(),
        errors: vec![],
        example: String::new(),
        internal: false,
        deprecated: None,
    };
    let mut project = empty_project("docpkg");
    project.docs = ProjectDocs {
        package: Some(IrPackageDoc {
            name: "docpkg".to_string(),
            desc: vec![],
            deprecated: Some("use v2".to_string()),
        }),
        decls: vec![
            decl(IrDocKind::Func, "f"),
            decl(IrDocKind::Sub, "s"),
            decl(IrDocKind::Type, "T"),
            decl(IrDocKind::Union, "U"),
            decl(IrDocKind::Enum, "E"),
            decl(IrDocKind::Resource, "R"),
        ],
    };
    let inner = encode_project(
        &project,
        &BinaryReprMetadata::new("docpkg".to_string(), "1.0.0".to_string()),
    );
    let path = temp_mfp(&wrap_mfp(&inner, "docpkg", "docpkg", "1.0.0"));
    let docs = read_package_docs(&path).expect("docs");
    assert_eq!(docs.decls.len(), 6, "every documented decl round-trips");
    assert!(docs.package.is_some());
    let _ = std::fs::remove_file(&path);
}
