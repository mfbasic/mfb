use super::*;
use crate::ast::Visibility;
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFunction, HirParam, HirTypeField,
};
use crate::types::ParameterType;

mod refinement;

fn subs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// String adapter over the native [`unify_type`], so the historical
/// string-level cases keep asserting the same behavior through the
/// `parse`↔`name` round-trip (which is byte-exact).
fn unify_str(
    pattern: &str,
    actual: &str,
    params: &[String],
    subs: &mut HashMap<String, String>,
) -> bool {
    let param_set: std::collections::HashSet<crate::intern::Symbol> = params
        .iter()
        .map(|p| crate::intern::Symbol::intern(p))
        .collect();
    let mut native: HashMap<crate::intern::Symbol, ParameterType> = subs
        .iter()
        .map(|(k, v)| (crate::intern::Symbol::intern(k), ParameterType::parse(v)))
        .collect();
    let ok = unify_type(
        &ParameterType::parse(pattern),
        &ParameterType::parse(actual),
        &param_set,
        &mut native,
    );
    *subs = native
        .iter()
        .map(|(k, v)| (k.resolve().to_string(), v.name().into_owned()))
        .collect();
    ok
}

/// String adapter over the native [`substitute_type_params`] (same rationale).
fn substitute_str(type_name: &str, subs: &HashMap<String, String>) -> String {
    let native: HashMap<crate::intern::Symbol, ParameterType> = subs
        .iter()
        .map(|(k, v)| (crate::intern::Symbol::intern(k), ParameterType::parse(v)))
        .collect();
    substitute_type_params(&ParameterType::parse(type_name), &native)
        .name()
        .into_owned()
}

fn param(name: &str, type_name: Option<&str>) -> HirParam {
    HirParam {
        name: name.to_string(),
        type_: type_name.map_or(ParameterType::Unknown, ParameterType::parse),
        resource: false,
        state_type: None,
        stray_state: None,
        default: None,
        line: 1,
    }
}

fn func(name: &str, params: Vec<HirParam>, return_type: Option<&str>) -> HirFunction {
    HirFunction {
        visibility: Visibility::Private,
        isolated: false,
        name: name.to_string(),
        template_params: Vec::new(),
        params,
        returns: return_type.map_or(ParameterType::Unknown, ParameterType::parse),
        return_resource: false,
        return_state_type: None,
        return_stray_state: None,
        body: Vec::new(),
        trap: None,
        line: 1,
        internal_origin: false,
        instantiated_at: Vec::new(),
    }
}

/// bug-752: a `List OF RES File` binds `T := RES File`, and a callback over its
/// elements declares `FUNC(File)`. The `RES` marker is ownership, not type, so
/// the two agree and the marked binding is kept — in either argument order.
#[test]
fn unify_agrees_modulo_the_res_marker() {
    let params = vec!["T".to_string(), "K".to_string()];
    let mut s = HashMap::new();
    assert!(unify_str(
        "List OF T",
        "List OF RES fs.File",
        &params,
        &mut s
    ));
    assert!(unify_str(
        "FUNC(T) AS K",
        "FUNC(fs.File) AS Integer",
        &params,
        &mut s
    ));
    assert_eq!(s.get("T").map(String::as_str), Some("RES fs.File"));
    let mut s = HashMap::new();
    assert!(unify_str(
        "FUNC(T) AS K",
        "FUNC(fs.File) AS Integer",
        &params,
        &mut s
    ));
    assert!(unify_str(
        "List OF T",
        "List OF RES fs.File",
        &params,
        &mut s
    ));
    assert_eq!(s.get("T").map(String::as_str), Some("RES fs.File"));
    // A genuinely different type still disagrees.
    assert!(!unify_str(
        "FUNC(T) AS K",
        "FUNC(String) AS Integer",
        &params,
        &mut s
    ));
}

#[test]
fn unify_binds_and_checks_template_params() {
    let params = vec!["T".to_string()];
    let mut s = HashMap::new();
    // First occurrence binds T -> Integer.
    assert!(unify_str("T", "Integer", &params, &mut s));
    assert_eq!(s.get("T").map(String::as_str), Some("Integer"));
    // Second occurrence must agree.
    assert!(unify_str("T", "Integer", &params, &mut s));
    // Conflicting binding fails.
    assert!(!unify_str("T", "String", &params, &mut s));
}

#[test]
fn unknown_actual_never_poisons_a_param_binding() {
    // bug-442: an `Unknown` actual (e.g. an empty `[]` literal types as
    // `List OF Unknown`) must not occupy a param slot and block a later
    // concrete binding, in EITHER field order.
    let params = vec!["T".to_string()];
    // Unknown first, concrete second.
    let mut s = HashMap::new();
    assert!(unify_str("List OF T", "List OF Unknown", &params, &mut s));
    assert!(unify_str(
        "FUNC(T) AS Boolean",
        "FUNC(Integer) AS Boolean",
        &params,
        &mut s
    ));
    assert_eq!(s.get("T"), Some(&"Integer".to_string()));
    // Concrete first, Unknown second — the concrete binding survives.
    let mut s2 = HashMap::new();
    assert!(unify_str(
        "FUNC(T) AS Boolean",
        "FUNC(Integer) AS Boolean",
        &params,
        &mut s2
    ));
    assert!(unify_str("List OF T", "List OF Unknown", &params, &mut s2));
    assert_eq!(s2.get("T"), Some(&"Integer".to_string()));
    // A genuine conflict between two concretes still fails.
    let mut s3 = HashMap::new();
    assert!(unify_str("T", "Integer", &params, &mut s3));
    assert!(!unify_str("T", "String", &params, &mut s3));
    // All-Unknown records a *provisional* `Unknown` binding (never dropped),
    // preserving the degenerate `T := Unknown` instantiation that width-agnostic
    // native ops such as `collections::flatten` rely on. A later concrete actual
    // still refines it (asserted above); with none, it stays Unknown.
    let mut s4 = HashMap::new();
    assert!(unify_str("List OF T", "List OF Unknown", &params, &mut s4));
    assert_eq!(s4.get("T"), Some(&"Unknown".to_string()));
    assert!(unify_str("T", "Integer", &params, &mut s4));
    assert_eq!(s4.get("T"), Some(&"Integer".to_string()));
}

#[test]
fn unify_recurses_into_all_container_shapes() {
    let params = vec!["T".to_string(), "U".to_string()];
    let cases = [
        ("List OF T", "List OF Integer"),
        ("Set OF T", "Set OF Integer"),
        ("#Result OF T", "#Result OF String"),
        ("Map OF T TO U", "Map OF String TO Integer"),
        ("MapEntry OF T TO U", "MapEntry OF String TO Integer"),
        ("Box OF T", "Box OF Integer"),
        ("FUNC(T) AS U", "FUNC(Integer) AS String"),
    ];
    for (pattern, actual) in cases {
        let mut s = HashMap::new();
        assert!(
            unify_str(pattern, actual, &params, &mut s),
            "unify {pattern} vs {actual}"
        );
    }
}

#[test]
fn unify_recurses_into_thread_shapes() {
    // Thread types unify by kind, message, optional resource, and output.
    let params = vec!["T".to_string(), "U".to_string()];
    let mut s = HashMap::new();
    assert!(unify_str(
        "Thread OF T TO U",
        "Thread OF Integer TO String",
        &params,
        &mut s
    ));
    // A thread with a resource clause on both sides unifies its resource slot.
    let mut s2 = HashMap::new();
    assert!(unify_str(
        "ThreadWorker OF T RES U TO Nothing",
        "ThreadWorker OF Integer RES String TO Nothing",
        &params,
        &mut s2
    ));
    // Resource present on one side only fails to unify.
    let mut s3 = HashMap::new();
    assert!(!unify_str(
        "Thread OF T RES U TO Nothing",
        "Thread OF Integer TO Nothing",
        &params,
        &mut s3
    ));
    // A thread pattern against a non-thread actual fails.
    let mut s4 = HashMap::new();
    assert!(!unify_str("Thread OF T TO U", "Integer", &params, &mut s4));
}

#[test]
fn unify_rejects_mismatched_container_shapes() {
    let params = vec!["T".to_string(), "U".to_string()];
    let cases = [
        ("List OF T", "Integer"),
        ("#Result OF T", "Integer"),
        ("Map OF T TO U", "Integer"),
        ("Map OF T TO U", "Map OF Integer"),
        ("MapEntry OF T TO U", "Integer"),
        ("MapEntry OF T TO U", "MapEntry OF Integer"),
        ("Box OF T", "Other OF Integer"),
        ("Box OF T, U", "Box OF Integer"),
        ("FUNC(T) AS U", "FUNC(Integer, String) AS Integer"),
        ("FUNC(T) AS U", "Integer"),
    ];
    for (pattern, actual) in cases {
        let mut s = HashMap::new();
        assert!(
            !unify_str(pattern, actual, &params, &mut s),
            "expected mismatch {pattern} vs {actual}"
        );
    }
}

#[test]
fn unify_treats_unknown_actual_as_wildcard_and_matches_concretes() {
    let params: Vec<String> = Vec::new();
    let mut s = HashMap::new();
    assert!(unify_str("Integer", "Integer", &params, &mut s));
    assert!(unify_str("Integer", "Unknown", &params, &mut s));
    assert!(!unify_str("Integer", "String", &params, &mut s));
}

/// plan-105-B retired monomorph's private `func_type_parts`; the canonical
/// grammar decomposes a function type into the same pieces (and, unlike the
/// private copy, preserves the `ISOLATED` marker instead of erasing it).
#[test]
fn func_type_parts_handles_isolated_and_empty_params() {
    assert_eq!(
        func_parts("FUNC(Integer, String) AS Boolean"),
        Some((
            vec!["Integer".to_string(), "String".to_string()],
            "Boolean".to_string(),
            false
        ))
    );
    assert_eq!(
        func_parts("ISOLATED FUNC() AS Nothing"),
        Some((Vec::new(), "Nothing".to_string(), true))
    );
    assert_eq!(func_parts("Integer"), None);
    assert_eq!(func_parts("FUNC(Integer)"), None);
}

/// The parameter/return decomposition of a `FUNC(...) AS R` spelling, read off
/// the canonical [`ParameterType::Func`].
fn func_parts(type_name: &str) -> Option<(Vec<String>, String, bool)> {
    match PARSE(type_name) {
        crate::types::ParameterType::Func(params, ret, isolated) => Some((
            params.iter().map(|p| p.name().into_owned()).collect(),
            ret.name().into_owned(),
            isolated,
        )),
        _ => None,
    }
}

/// The template head and rendered type arguments of a user generic, read off the
/// canonical [`ParameterType::UserOf`] — the successor of the deleted
/// `user_template_parts`.
fn user_parts(type_name: &str) -> Option<(String, Vec<String>)> {
    match PARSE(type_name) {
        crate::types::ParameterType::UserOf(name, args) => Some((
            name.resolve().to_string(),
            args.iter().map(|a| a.name().into_owned()).collect(),
        )),
        _ => None,
    }
}

#[allow(non_snake_case)]
fn PARSE(type_name: &str) -> crate::types::ParameterType {
    crate::types::ParameterType::parse(type_name)
}

/// bug-35: a type argument that is itself a comma-bearing function type must
/// survive the split, or unification and mangling operate on garbage.
#[test]
fn nested_function_type_arguments_are_not_shredded() {
    assert_eq!(
        func_parts("FUNC(FUNC(Integer, String) AS Boolean, Integer) AS Nothing"),
        Some((
            vec![
                "FUNC(Integer, String) AS Boolean".to_string(),
                "Integer".to_string()
            ],
            "Nothing".to_string(),
            false
        ))
    );
    assert_eq!(
        func_parts("ISOLATED FUNC(FUNC(A, B) AS C) AS D"),
        Some((vec!["FUNC(A, B) AS C".to_string()], "D".to_string(), true))
    );
    // A two-argument template whose first argument is a two-parameter FUNC.
    assert_eq!(
        user_parts("Pair OF FUNC(Integer, String) AS Boolean, Integer"),
        Some((
            "Pair".to_string(),
            vec![
                "FUNC(Integer, String) AS Boolean".to_string(),
                "Integer".to_string()
            ]
        ))
    );
    // The depth-aware top-level comma split now lives in the canonical grammar.
    assert_eq!(
        crate::codegen::builtins::split_top_level_commas("Pair OF Integer, String"),
        vec!["Pair OF Integer", "String"]
    );
    assert_eq!(
        crate::codegen::builtins::split_top_level_commas("FUNC(A, B) AS C, D"),
        vec!["FUNC(A, B) AS C", "D"]
    );
}

/// Substitution walks the type arguments the depth-aware split produces, so a
/// nested function-typed argument no longer swallows the argument after it.
#[test]
fn substitution_walks_each_top_level_type_argument() {
    let mut substitutions = HashMap::new();
    substitutions.insert("T".to_string(), "Integer".to_string());
    assert_eq!(
        substitute_str("Pair OF (List OF T, T)", &substitutions),
        "Pair OF (List OF Integer, Integer)"
    );
    assert_eq!(
        substitute_str("List OF T", &substitutions),
        "List OF Integer"
    );
}

/// A built-in constructor spelled with ` OF ` must never be read as a user
/// generic named `List`/`Map`/… — the ordering rule `ParameterType::parse`'s
/// `UserOf` arm inherited from the deleted `user_template_parts`.
#[test]
fn user_template_parts_excludes_builtin_shapes() {
    assert_eq!(
        user_parts("Pair OF Integer, String"),
        Some((
            "Pair".to_string(),
            vec!["Integer".to_string(), "String".to_string()]
        ))
    );
    for builtin in [
        "List OF Integer",
        "Set OF Integer",
        "Map OF Integer TO String",
        "MapEntry OF Integer TO String",
        "#Result OF Integer",
        "Thread OF Integer TO String",
        "ThreadWorker OF Integer TO String",
        "FUNC(Integer) AS String",
        "ISOLATED FUNC() AS Nothing",
    ] {
        assert_eq!(user_parts(builtin), None, "{builtin}");
    }
    assert_eq!(user_parts("Integer"), None);
}

#[test]
fn substitute_type_params_rewrites_every_shape() {
    let s = subs(&[("T", "Integer"), ("U", "String")]);
    assert_eq!(substitute_str("T", &s), "Integer");
    assert_eq!(substitute_str("List OF T", &s), "List OF Integer");
    assert_eq!(substitute_str("Set OF T", &s), "Set OF Integer");
    assert_eq!(substitute_str("#Result OF T", &s), "#Result OF Integer");
    assert_eq!(
        substitute_str("Map OF T TO U", &s),
        "Map OF Integer TO String"
    );
    assert_eq!(
        substitute_str("MapEntry OF T TO U", &s),
        "MapEntry OF Integer TO String"
    );
    assert_eq!(
        substitute_str("Pair OF (T, U)", &s),
        "Pair OF (Integer, String)"
    );
    // Thread shape substitutes message and output slots.
    assert_eq!(
        substitute_str("Thread OF T TO U", &s),
        "Thread OF Integer TO String"
    );
    // Unknown names pass through unchanged.
    assert_eq!(substitute_str("Boolean", &s), "Boolean");
    // Malformed Map (no TO) falls through to the identity return.
    assert_eq!(substitute_str("Map OF T", &s), "Map OF T");
    // Malformed MapEntry (no TO) also falls through.
    assert_eq!(substitute_str("MapEntry OF T", &s), "MapEntry OF T");
    // A callable substitutes its parameters and return, keeping isolation.
    assert_eq!(
        substitute_str("FUNC(T, List OF U) AS T", &s),
        "FUNC(Integer, List OF String) AS Integer"
    );
    assert_eq!(
        substitute_str("ISOLATED FUNC(T) AS Nothing", &s),
        "ISOLATED FUNC(Integer) AS Nothing"
    );
}

/// bug-822: a reference to an overload member is selected by its complete
/// callable type — parameters, return and isolation.
#[test]
fn callable_matches_compares_the_complete_callable_type() {
    let t = ParameterType::parse;
    let plain = t("FUNC(Integer) AS Integer");
    let isolated = t("ISOLATED FUNC(Integer) AS Integer");
    assert!(callable_matches(&plain, &plain));
    // An isolated member widens to a plain slot; the reverse does not.
    assert!(callable_matches(&plain, &isolated));
    assert!(!callable_matches(&isolated, &plain));
    assert!(callable_matches(&isolated, &isolated));
    // Return type and every parameter type count; so does arity.
    assert!(!callable_matches(&t("FUNC(Integer) AS String"), &plain));
    assert!(!callable_matches(&t("FUNC(String) AS Integer"), &plain));
    assert!(!callable_matches(
        &t("FUNC(Integer, Integer) AS Integer"),
        &plain
    ));
    // A top-level RES marker on a parameter is not part of the identity.
    assert!(callable_matches(
        &t("FUNC(RES fs.File) AS Integer"),
        &t("FUNC(fs.File) AS Integer")
    ));
    // A non-callable expected type selects nothing.
    assert!(!callable_matches(&ParameterType::Integer, &plain));
}

#[test]
fn reference_signature_defaults_a_missing_return_to_nothing() {
    let declared = func("f", vec![param("a", Some("String"))], Some("Integer"));
    assert_eq!(
        reference_signature(&declared).name(),
        "FUNC(String) AS Integer"
    );
    let effect = func("g", vec![param("a", Some("String"))], None);
    assert_eq!(
        reference_signature(&effect).name(),
        "FUNC(String) AS Nothing"
    );
}

#[test]
fn arg_slot_expected_for_calls_and_bare_names() {
    let params = vec![param("f", Some("FUNC(Integer) AS Integer"))];
    let reference = HirExpression::Identifier("pick".to_string());
    assert_eq!(
        arg_slot_expected(&reference, Some(&params), |p| p.first()),
        Some(ParameterType::parse("FUNC(Integer) AS Integer"))
    );
    assert!(consults_expected(&reference));
    assert!(!consults_expected(&HirExpression::Number("1".to_string())));
}

#[test]
fn mangle_and_sanitize_encode_types() {
    assert_eq!(mangle_name("push", &["Integer".into()]), "push$Integer");
    assert_eq!(
        mangle_name("f", &["List OF Integer".into(), "String".into()]),
        "f$List$OF$Integer$String"
    );
    assert_eq!(sanitize_type_name("Map OF K TO V"), "Map$OF$K$TO$V");
    assert_eq!(sanitize_type_name("plain_1"), "plain_1");
}

#[test]
fn overload_concrete_name_encodes_params_and_return() {
    let f = func("g", vec![param("a", Some("Integer"))], Some("String"));
    // Neither overloaded nor return-disambiguated: bare name.
    assert_eq!(overload_concrete_name(&f, false, false), "g");
    // Overloaded by params only.
    assert_eq!(overload_concrete_name(&f, true, false), "g$Integer");
    // Return-disambiguated appends the AS <return> segment.
    assert_eq!(
        overload_concrete_name(&f, true, true),
        "g$Integer$AS$String"
    );
    // Missing param/return types fall back to Unknown/Nothing.
    let bare = func("h", vec![param("a", None)], None);
    assert_eq!(
        overload_concrete_name(&bare, true, true),
        "h$Unknown$AS$Nothing"
    );
}

#[test]
fn overload_key_includes_return_type() {
    let params = vec![param("a", Some("Integer")), param("b", None)];
    assert_eq!(
        overload_key("f", &params, Some("Boolean")),
        "f(Integer,Unknown) AS Boolean"
    );
    assert_eq!(overload_key("f", &[], None), "f() AS Nothing");
}

#[test]
fn param_types_eq_and_params_match() {
    let a = func("f", vec![param("x", Some("Integer"))], None);
    let b = func("f", vec![param("y", Some("Integer"))], Some("String"));
    let c = func("f", vec![param("z", Some("String"))], None);
    assert!(param_types_eq(&a, &b));
    assert!(!param_types_eq(&a, &c));
    assert!(params_match(&a, &[ParameterType::Integer]));
    assert!(!params_match(&a, &[ParameterType::String]));
    assert!(!params_match(&a, &[]));
}

#[test]
fn arg_slot_expected_only_for_call_arguments() {
    let params = [param("a", Some("Integer"))];
    let call = HirExpression::Call {
        callee: "f".to_string(),
        arguments: Vec::new(),
        line: 1,
        column: 1,
    };
    assert_eq!(
        arg_slot_expected(&call, Some(&params), |p| p.first()),
        Some(ParameterType::Integer)
    );
    // Non-call arguments get no contextual type.
    let lit = HirExpression::Number("1".to_string());
    assert_eq!(arg_slot_expected(&lit, Some(&params), |p| p.first()), None);
    // No params available.
    assert_eq!(arg_slot_expected(&call, None, |p| p.first()), None);
}

#[test]
fn constructor_arg_field_type_positional_and_named() {
    let fields = [
        HirTypeField {
            visibility: None,
            name: "x".to_string(),
            type_: ParameterType::parse("Integer"),
            line: 1,
        },
        HirTypeField {
            visibility: None,
            name: "y".to_string(),
            type_: ParameterType::parse("String"),
            line: 1,
        },
    ];
    let pos = HirConstructorArg::Positional(HirExpression::Number("1".to_string()));
    assert_eq!(
        constructor_arg_field_type(&pos, 1, Some(&fields)),
        Some(ParameterType::String)
    );
    let named = HirConstructorArg::Named {
        name: "x".to_string(),
        value: HirExpression::Number("1".to_string()),
        line: 1,
    };
    assert_eq!(
        constructor_arg_field_type(&named, 0, Some(&fields)),
        Some(ParameterType::Integer)
    );
    // No fields known.
    assert_eq!(constructor_arg_field_type(&pos, 0, None), None);
}

#[test]
fn arg_and_constructor_value_accessors() {
    let pos = HirCallArg::Positional(HirExpression::Number("1".to_string()));
    let named = HirCallArg::Named {
        name: "a".to_string(),
        value: HirExpression::Number("2".to_string()),
        line: 1,
    };
    assert!(matches!(call_arg_value(&pos), HirExpression::Number(n) if n == "1"));
    assert!(matches!(call_arg_value(&named), HirExpression::Number(n) if n == "2"));
    let cpos = HirConstructorArg::Positional(HirExpression::Number("3".to_string()));
    let cnamed = HirConstructorArg::Named {
        name: "a".to_string(),
        value: HirExpression::Number("4".to_string()),
        line: 1,
    };
    assert!(matches!(constructor_arg_value(&cpos), HirExpression::Number(n) if n == "3"));
    assert!(matches!(constructor_arg_value(&cnamed), HirExpression::Number(n) if n == "4"));
}

#[test]
fn numeric_result_and_loop_promotion() {
    // plan-106-A deleted monomorph's `numeric_binary_result_type` wrapper;
    // the engine calls the one typed source directly.
    assert_eq!(
        crate::numeric::typed_binary_result_type(
            BinaryOp::Add,
            &ParameterType::Integer,
            &ParameterType::Integer
        ),
        Some(ParameterType::Integer)
    );
    assert_eq!(
        crate::numeric::typed_binary_result_type(
            BinaryOp::Add,
            &ParameterType::Integer,
            &ParameterType::Float
        ),
        Some(ParameterType::Float)
    );
    // A Float bound anywhere in a FOR loop promotes the counter type.
    assert_eq!(
        crate::numeric::typed_promote_loop_numeric_type(
            &ParameterType::Integer,
            &ParameterType::Float,
            &ParameterType::Integer
        ),
        ParameterType::Float
    );
    assert_eq!(
        crate::numeric::typed_promote_loop_numeric_type(
            &ParameterType::Integer,
            &ParameterType::Integer,
            &ParameterType::Integer
        ),
        ParameterType::Integer
    );
}

/// plan-105-B moved both splitters into the canonical grammar
/// (`crate::types::split_top_level_to`, `builtins::split_top_level_commas`);
/// these assertions follow them there.
#[test]
fn split_helpers() {
    assert_eq!(
        crate::types::split_top_level_to("Integer TO String"),
        Some(("Integer", "String"))
    );
    assert_eq!(crate::types::split_top_level_to("Integer"), None);
    assert_eq!(
        crate::codegen::builtins::split_top_level_commas("Integer, String"),
        vec!["Integer", "String"]
    );
}

#[test]
fn collect_imported_overloads_empty_without_imports() {
    // A project with no import bindings and no packages directory yields no
    // overloads and no qualifiers.
    let dir = std::env::temp_dir();
    let project = crate::ast::AstProject {
        name: "p".to_string(),
        files: Vec::new(),
    };
    let (overloads, qualifiers) =
        collect_imported_overloads(&dir, &crate::hir::elaborate(&project));
    assert!(overloads.is_empty());
    assert!(qualifiers.is_empty());
}

#[test]
fn collect_imported_overloads_reads_package_overload_set() {
    // Import a real compiled package whose exports include overload sets
    // (`score$`/`score$Vec2`, `mark$`/`mark$Vec2`) so the by-base grouping,
    // the ≥2 overload gate, and the qualifier collection all run.
    let fixture = crate::testutil::fixture_dir("package-simple")
        .join("golden")
        .join("package_simple.mfp");
    let dir = tempfile::tempdir().expect("tempdir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();

    let src = "IMPORT package_simple\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";
    let file = crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
        .expect("parse");
    let project = crate::ast::AstProject {
        name: "app".to_string(),
        files: vec![file],
    };

    let (overloads, qualifiers) =
        collect_imported_overloads(dir.path(), &crate::hir::elaborate(&project));
    // Overload sets are keyed by `package.base` (bug-914).
    assert!(
        overloads.contains_key("package_simple.score"),
        "keys: {:?}",
        overloads.keys().collect::<Vec<_>>()
    );
    let score = &overloads["package_simple.score"];
    assert!(score.len() >= 2);
    // Each collected overload carries the export's mangled name.
    assert!(score.iter().all(|o| o.export_name.starts_with("score")));
    // bug-632: a USER package's qualifier is NOT captured — its types are
    // package-qualified on both sides of the overload match, so stripping the
    // qualifier would merge two packages' same-named types. Only a built-in
    // import contributes a qualifier.
    assert!(
        !qualifiers.iter().any(|q| q == "package_simple."),
        "a user package's qualifier must not be stripped: {qualifiers:?}"
    );
    // Each candidate's parameter types carry the package's identity, so they
    // compare equal to an argument the parser canonicalized the same way.
    assert!(
        score
            .iter()
            .all(|overload| overload.param_types.iter().all(
                |type_| !matches!(type_, ParameterType::Named(sym) if sym.resolve() == "Vec2")
            )),
        "an owned nominal parameter is package-qualified: {:?}",
        score
            .iter()
            .map(|o| o.param_types.iter().map(|t| t.name()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
}
