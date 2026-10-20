use super::*;

/// plan-111-C: `resolve_return_type` takes and returns types now.
fn types(items: &[&str]) -> Vec<ParameterType> {
    items.iter().map(|s| ParameterType::parse(s)).collect()
}

fn rt(name: &str, args: &[&str]) -> Option<String> {
    // The assertions read as spellings on purpose — a resolver test says
    // which type a call resolves to, and a name is how it says it. The parse
    // is here, once, at the test boundary (plan-111-C Correction C4's rule).
    resolve_call(name, &types(args), &NoTypeKinds).map(|r| r.return_type.name().into_owned())
}

/// A FUNC signature for `builtin_function_id_for_type`, parsed at the same
/// test boundary.
fn ft(spelling: &str) -> ParameterType {
    ParameterType::parse(spelling)
}

const ALL_GENERAL: &[&str] = &[
    ERROR,
    LEN,
    TYPE_NAME,
    TO_STRING,
    TO_INT,
    TO_FLOAT,
    TO_FIXED,
    TO_BYTE,
    TO_MONEY,
    TO_SCALAR,
    IS_NUMERIC,
    IS_EVEN,
    IS_ODD,
    IS_POSITIVE,
    IS_NEGATIVE,
    IS_ZERO,
    IS_EMPTY,
    IS_NOT_EMPTY,
];

#[test]
fn is_general_call_covers_all() {
    for name in ALL_GENERAL {
        assert!(is_general_call(name), "{name}");
    }
    assert_eq!(ALL_GENERAL.len(), 18);
    assert!(!is_general_call("nope"));
    assert!(!is_general_call("collections.get"));
}

/// bug-569 load-bearing invariant: **no builtin that can be passed directly
/// as a callback returns a `String`.**
///
/// A built-in used as a `FunctionRef` is the one callback source that is not
/// in `module.functions` at all, so it is invisible to
/// `function_returns_fresh_string` — the promise every other callback source
/// makes about owning the block it returns (bug-562). The HOFs now FREE that
/// block (`collections::transform`, `sortBy`, `groupBy`, `mapValues`), and
/// the only reason a builtin cannot break that is that the whole admitted set
/// is `Boolean`-returning predicates, which allocate nothing.
///
/// That was a fact about the list rather than an invariant, and the free made
/// it load-bearing. It is an invariant now: admit a `String`-returning name to
/// `builtin_function_id` and this goes red, instead of the HOF freeing a block
/// it does not own.
#[test]
fn every_builtin_that_can_be_a_callback_returns_boolean() {
    const ARGUMENT_TYPES: &[&str] = &[
        "Integer",
        "Float",
        "Fixed",
        "Money",
        "Byte",
        "Scalar",
        "Boolean",
        "String",
        "List OF Integer",
        "List OF String",
        "Set OF Integer",
        "Map OF String TO Integer",
    ];
    let admitted: Vec<&&str> = ALL_GENERAL
        .iter()
        .filter(|name| builtin_function_id(name).is_some())
        .collect();
    assert_eq!(
        admitted.len(),
        8,
        "the callback-capable builtin set changed: {admitted:?}. Every member \
             must be re-checked against the HOF free in bug-569 before this count \
             is updated"
    );
    for name in admitted {
        for spelling in ARGUMENT_TYPES {
            let Some(resolved) = resolve_call(name, &types(&[spelling]), &NoTypeKinds) else {
                continue;
            };
            assert_eq!(
                resolved.return_type,
                ParameterType::Boolean,
                "{name}({spelling}) returns {} — a builtin reachable as a \
                     `FunctionRef` must return `Boolean`, because it makes no \
                     promise about owning a returned block and the HOFs free one",
                resolved.return_type.name()
            );
        }
    }
}

#[test]
fn overridable_and_reserved() {
    assert!(!is_overridable(ERROR));
    assert!(reserved_builtin_name(ERROR));
    for name in ALL_GENERAL.iter().filter(|n| **n != ERROR) {
        assert!(is_overridable(name), "{name}");
        assert!(!reserved_builtin_name(name), "{name}");
    }
    assert!(!is_overridable("nope"));
    assert!(!reserved_builtin_name("nope"));
}

#[test]
fn override_result_type_all_arms() {
    assert_eq!(override_result_type(TO_STRING), Some("String"));
    assert_eq!(override_result_type(TYPE_NAME), Some("String"));
    assert_eq!(override_result_type(LEN), Some("Integer"));
    assert_eq!(override_result_type(TO_INT), Some("Integer"));
    assert_eq!(override_result_type(TO_FLOAT), Some("Float"));
    assert_eq!(override_result_type(TO_FIXED), Some("Fixed"));
    assert_eq!(override_result_type(TO_BYTE), Some("Byte"));
    assert_eq!(override_result_type(TO_MONEY), Some("Money"));
    assert_eq!(override_result_type(TO_SCALAR), Some("Scalar"));
    assert_eq!(override_result_type(IS_NUMERIC), Some("Boolean"));
    assert_eq!(override_result_type(IS_NOT_EMPTY), Some("Boolean"));
    assert_eq!(override_result_type(ERROR), None);
    assert_eq!(override_result_type("nope"), None);
}

#[test]
fn call_param_names_all_arms() {
    assert_eq!(call_param_names(ERROR).unwrap().len(), 2);
    assert_eq!(call_param_names(LEN).unwrap().len(), 1);
    assert_eq!(call_param_names(TYPE_NAME).unwrap().len(), 1);
    assert_eq!(call_param_names(TO_STRING).unwrap().len(), 2);
    assert_eq!(call_param_names(TO_INT).unwrap().len(), 2);
    assert_eq!(call_param_names(TO_FLOAT).unwrap().len(), 1);
    assert_eq!(call_param_names(TO_FIXED).unwrap().len(), 1);
    assert_eq!(call_param_names(TO_BYTE).unwrap().len(), 1);
    assert_eq!(call_param_names(TO_MONEY), Some(&[&["value"][..]][..]));
    assert_eq!(call_param_names(TO_SCALAR), Some(&[&["value"][..]][..]));
    assert_eq!(call_param_names(IS_NUMERIC).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_EVEN).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_ODD).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_POSITIVE).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_NEGATIVE).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_ZERO).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_EMPTY).unwrap().len(), 1);
    assert_eq!(call_param_names(IS_NOT_EMPTY).unwrap().len(), 1);
    assert!(call_param_names("nope").is_none());
}

#[test]
fn builtin_function_id_arms() {
    assert_eq!(builtin_function_id(IS_EVEN), Some(BUILTIN_FUNCTION_IS_EVEN));
    assert_eq!(builtin_function_id(IS_ODD), Some(BUILTIN_FUNCTION_IS_ODD));
    assert_eq!(
        builtin_function_id(IS_POSITIVE),
        Some(BUILTIN_FUNCTION_IS_POSITIVE)
    );
    assert_eq!(
        builtin_function_id(IS_NEGATIVE),
        Some(BUILTIN_FUNCTION_IS_NEGATIVE)
    );
    assert_eq!(builtin_function_id(IS_ZERO), Some(BUILTIN_FUNCTION_IS_ZERO));
    assert_eq!(
        builtin_function_id(IS_EMPTY),
        Some(BUILTIN_FUNCTION_IS_EMPTY)
    );
    assert_eq!(
        builtin_function_id(IS_NOT_EMPTY),
        Some(BUILTIN_FUNCTION_IS_NOT_EMPTY)
    );
    assert_eq!(
        builtin_function_id(IS_NUMERIC),
        Some(BUILTIN_FUNCTION_IS_NUMERIC)
    );
    assert_eq!(builtin_function_id(LEN), None);
    assert_eq!(builtin_function_id("nope"), None);
}

#[test]
fn builtin_function_id_for_type_specialized() {
    assert_eq!(
        builtin_function_id_for_type(IS_POSITIVE, &ft("FUNC(Float) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_POSITIVE, &ft("FUNC(Fixed) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_NEGATIVE, &ft("FUNC(Float) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_NEGATIVE, &ft("FUNC(Fixed) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_ZERO, &ft("FUNC(Float) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_ZERO, &ft("FUNC(Fixed) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_ZERO_FIXED)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_POSITIVE, &ft("FUNC(Integer) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_POSITIVE)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_EVEN, &ft("FUNC(Integer) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_EVEN)
    );
}

#[test]
fn builtin_function_id_for_type_non_predicate_shape() {
    assert_eq!(
        builtin_function_id_for_type(IS_EVEN, &ft("FUNC(Integer, Integer) AS Boolean")),
        Some(BUILTIN_FUNCTION_IS_EVEN)
    );
    assert_eq!(
        builtin_function_id_for_type(IS_EVEN, &ft("FUNC(Integer) AS Integer")),
        Some(BUILTIN_FUNCTION_IS_EVEN)
    );
    assert_eq!(builtin_function_id_for_type(IS_EVEN, &ft("Integer")), None);
}

#[test]
fn filter_predicate_type_cases() {
    assert_eq!(
        filter_predicate_type_typed(IS_EVEN, &ParameterType::Integer),
        Some(ft("FUNC(Integer) AS Boolean"))
    );
    assert_eq!(
        filter_predicate_type_typed(IS_POSITIVE, &ParameterType::Float),
        Some(ft("FUNC(Float) AS Boolean"))
    );
    assert_eq!(
        filter_predicate_type_typed(LEN, &ParameterType::String),
        None
    );
    assert_eq!(
        filter_predicate_type_typed(IS_EVEN, &ParameterType::String),
        None
    );
}

#[test]
fn resolve_error() {
    assert_eq!(rt(ERROR, &["Integer", "String"]), Some("Error".to_string()));
    assert_eq!(rt(ERROR, &["String", "Integer"]), None);
    assert_eq!(rt(ERROR, &["Integer"]), None);
}

#[test]
fn resolve_len() {
    assert_eq!(rt(LEN, &["String"]), Some("Integer".to_string()));
    assert_eq!(rt(LEN, &["List OF Integer"]), Some("Integer".to_string()));
    assert_eq!(
        rt(LEN, &["Map OF String TO Integer"]),
        Some("Integer".to_string())
    );
    assert_eq!(rt(LEN, &["Integer"]), None);
    assert_eq!(rt(LEN, &["String", "String"]), None);
}

#[test]
fn resolve_type_name() {
    assert_eq!(rt(TYPE_NAME, &["Anything"]), Some("String".to_string()));
    assert_eq!(rt(TYPE_NAME, &["a", "b"]), None);
}

#[test]
fn resolve_to_string() {
    assert_eq!(
        rt(TO_STRING, &["Float", "Byte"]),
        Some("String".to_string())
    );
    assert_eq!(
        rt(TO_STRING, &["Fixed", "Byte"]),
        Some("String".to_string())
    );
    assert_eq!(rt(TO_STRING, &["Integer"]), Some("String".to_string()));
    assert_eq!(rt(TO_STRING, &["Boolean"]), Some("String".to_string()));
    assert_eq!(rt(TO_STRING, &["List OF Byte"]), Some("String".to_string()));
    assert_eq!(rt(TO_STRING, &["Integer", "Byte"]), None);
    assert_eq!(rt(TO_STRING, &["Float", "Integer"]), None);
    assert_eq!(rt(TO_STRING, &["List OF Integer"]), None);
}

#[test]
fn resolve_to_int() {
    assert_eq!(rt(TO_INT, &["String"]), Some("Integer".to_string()));
    assert_eq!(rt(TO_INT, &["Byte"]), Some("Integer".to_string()));
    assert_eq!(rt(TO_INT, &["Float"]), Some("Integer".to_string()));
    assert_eq!(rt(TO_INT, &["Fixed"]), Some("Integer".to_string()));
    assert_eq!(
        rt(TO_INT, &["String", "Integer"]),
        Some("Integer".to_string())
    );
    assert_eq!(rt(TO_INT, &["Boolean"]), None);
    assert_eq!(rt(TO_INT, &["Integer", "Integer"]), None);
}

/// An oracle that calls every type an enum.
struct AllEnums;
impl TypeKinds for AllEnums {
    fn is_enum(&self, _t: &ParameterType) -> bool {
        true
    }
}

/// An oracle that knows exactly one enum, `Color`.
struct ColorIsAnEnum;
impl TypeKinds for ColorIsAnEnum {
    fn is_enum(&self, t: &ParameterType) -> bool {
        t.is_named("Color")
    }
}

fn rt_kinds(name: &str, args: &[&str], kinds: &dyn TypeKinds) -> Option<String> {
    resolve_call(name, &types(args), kinds).map(|r| r.return_type.name().into_owned())
}

/// plan-140-B: `toInt` accepts one enum argument, and only when the oracle
/// says it is one; the `base` form stays `String`-only.
#[test]
fn resolve_to_int_enum() {
    assert_eq!(
        rt_kinds(TO_INT, &["Color"], &ColorIsAnEnum),
        Some("Integer".to_string())
    );
    assert_eq!(rt_kinds(TO_INT, &["Color"], &NoTypeKinds), None);
    assert_eq!(
        rt_kinds(TO_INT, &["Color", "Integer"], &ColorIsAnEnum),
        None
    );
    assert_eq!(rt_kinds(TO_INT, &["Shape"], &ColorIsAnEnum), None);
}

/// plan-140-C: `toString` accepts one enum argument, and only when the
/// oracle says it is one; the precision form stays `Float`/`Fixed`/`Money`.
#[test]
fn resolve_to_string_enum() {
    assert_eq!(
        rt_kinds(TO_STRING, &["Color"], &ColorIsAnEnum),
        Some("String".to_string())
    );
    assert_eq!(rt_kinds(TO_STRING, &["Color"], &NoTypeKinds), None);
    assert_eq!(
        rt_kinds(TO_STRING, &["Color", "Byte"], &ColorIsAnEnum),
        None
    );
    assert_eq!(rt_kinds(TO_STRING, &["Shape"], &ColorIsAnEnum), None);
}

/// plan-140-A's seam reaches no built-in but `toInt` and `toString`: every
/// other case resolves the same under an always-true and an always-false
/// oracle.
#[test]
fn only_to_int_and_to_string_consult_the_kind_oracle() {
    let cases: &[&[&str]] = &[
        &["String"],
        &["Integer"],
        &["Boolean"],
        &["List OF Byte"],
        &["Color"],
        &["Float", "Byte"],
        &["Color", "Byte"],
    ];
    for name in [
        TO_FLOAT, TO_FIXED, TO_BYTE, TO_MONEY, TO_SCALAR, LEN, TYPE_NAME,
    ] {
        for args in cases {
            let with = |kinds: &dyn TypeKinds| {
                resolve_call(name, &types(args), kinds).map(|r| r.return_type)
            };
            assert_eq!(with(&AllEnums), with(&NoTypeKinds), "{name}({args:?})");
        }
    }
}

#[test]
fn resolve_to_float_fixed_byte() {
    assert_eq!(rt(TO_FLOAT, &["String"]), Some("Float".to_string()));
    assert_eq!(rt(TO_FLOAT, &["Integer"]), Some("Float".to_string()));
    assert_eq!(rt(TO_FLOAT, &["Fixed"]), Some("Float".to_string()));
    assert_eq!(rt(TO_FLOAT, &["Boolean"]), None);
    assert_eq!(rt(TO_FIXED, &["String"]), Some("Fixed".to_string()));
    assert_eq!(rt(TO_FIXED, &["Integer"]), Some("Fixed".to_string()));
    assert_eq!(rt(TO_FIXED, &["Float"]), Some("Fixed".to_string()));
    assert_eq!(rt(TO_FIXED, &["Boolean"]), None);
    assert_eq!(rt(TO_BYTE, &["Integer"]), Some("Byte".to_string()));
    assert_eq!(rt(TO_BYTE, &["String"]), None);
}

#[test]
fn resolve_to_scalar_and_money() {
    assert_eq!(rt(TO_SCALAR, &["Integer"]), Some("Scalar".to_string()));
    assert_eq!(rt(TO_SCALAR, &["String"]), Some("Scalar".to_string()));
    assert_eq!(rt(TO_SCALAR, &["Byte"]), Some("Scalar".to_string()));
    assert_eq!(rt(TO_SCALAR, &["Float"]), None);
    assert_eq!(rt(TO_SCALAR, &["Integer", "Integer"]), None);
    assert_eq!(rt(TO_MONEY, &["String"]), Some("Money".to_string()));
    assert_eq!(rt(TO_MONEY, &["Integer"]), Some("Money".to_string()));
    assert_eq!(rt(TO_MONEY, &["Float"]), Some("Money".to_string()));
    assert_eq!(rt(TO_MONEY, &["Fixed"]), Some("Money".to_string()));
    assert_eq!(rt(TO_MONEY, &["Byte"]), Some("Money".to_string()));
    assert_eq!(rt(TO_MONEY, &["Boolean"]), None);
}

#[test]
fn resolve_predicates() {
    assert_eq!(rt(IS_NUMERIC, &["String"]), Some("Boolean".to_string()));
    assert_eq!(rt(IS_NUMERIC, &["Integer"]), None);
    assert_eq!(rt(IS_EVEN, &["Integer"]), Some("Boolean".to_string()));
    assert_eq!(rt(IS_ODD, &["Integer"]), Some("Boolean".to_string()));
    assert_eq!(rt(IS_EVEN, &["Float"]), None);
    assert_eq!(rt(IS_POSITIVE, &["Integer"]), Some("Boolean".to_string()));
    assert_eq!(rt(IS_NEGATIVE, &["Float"]), Some("Boolean".to_string()));
    assert_eq!(rt(IS_ZERO, &["Fixed"]), Some("Boolean".to_string()));
    assert_eq!(rt(IS_POSITIVE, &["String"]), None);
    assert_eq!(rt(IS_EMPTY, &["String"]), Some("Boolean".to_string()));
    assert_eq!(
        rt(IS_EMPTY, &["List OF Integer"]),
        Some("Boolean".to_string())
    );
    assert_eq!(
        rt(IS_NOT_EMPTY, &["Map OF String TO Integer"]),
        Some("Boolean".to_string())
    );
    assert_eq!(rt(IS_NOT_EMPTY, &["Integer"]), None);
}

#[test]
fn resolve_call_unknown() {
    assert_eq!(rt("nope", &["Integer"]), None);
}

#[test]
fn resolve_return_type_wrapper_delegates() {
    assert_eq!(
        resolve_return_type(TO_MONEY, &types(&["Integer"]), &NoTypeKinds),
        Some(ParameterType::Money)
    );
    assert_eq!(
        resolve_return_type("nope", &types(&["Integer"]), &NoTypeKinds),
        None
    );
}

#[test]
fn nominal_return_type_matches_fast_oracle() {
    // The six numeric narrowing conversions carry a fixed return type.
    assert_eq!(nominal_return_type(TO_INT), Some(ParameterType::Integer));
    assert_eq!(nominal_return_type(TO_FLOAT), Some(ParameterType::Float));
    assert_eq!(nominal_return_type(TO_FIXED), Some(ParameterType::Fixed));
    assert_eq!(nominal_return_type(TO_BYTE), Some(ParameterType::Byte));
    assert_eq!(nominal_return_type(TO_MONEY), Some(ParameterType::Money));
    assert_eq!(
        nominal_return_type(TO_SCALAR),
        Some(ParameterType::named("Scalar"))
    );
    // Every other general call (Custom / reserved) has no static nominal.
    assert_eq!(nominal_return_type(LEN), None);
    assert_eq!(nominal_return_type(TYPE_NAME), None);
    assert_eq!(nominal_return_type(TO_STRING), None);
    assert_eq!(nominal_return_type(IS_EVEN), None);
    assert_eq!(nominal_return_type(ERROR), None);
    assert_eq!(nominal_return_type("nope"), None);
}

#[test]
fn expected_arguments_all_arms() {
    assert!(expected_arguments(LEN).is_some());
    assert!(expected_arguments(TYPE_NAME).is_some());
    assert!(expected_arguments(TO_STRING).is_some());
    assert!(expected_arguments(TO_INT).is_some());
    assert!(expected_arguments(TO_FLOAT).is_some());
    assert!(expected_arguments(TO_FIXED).is_some());
    assert!(expected_arguments(TO_BYTE).is_some());
    assert_eq!(
        expected_arguments(TO_MONEY),
        Some("String, Integer, Float, Fixed, or Byte")
    );
    assert_eq!(
        expected_arguments(TO_SCALAR),
        Some("Integer, String, or Byte")
    );
    assert!(expected_arguments(IS_NUMERIC).is_some());
    assert!(expected_arguments(IS_EVEN).is_some());
    assert!(expected_arguments(IS_ODD).is_some());
    assert!(expected_arguments(IS_POSITIVE).is_some());
    assert!(expected_arguments(IS_NEGATIVE).is_some());
    assert!(expected_arguments(IS_ZERO).is_some());
    assert!(expected_arguments(IS_EMPTY).is_some());
    assert!(expected_arguments(IS_NOT_EMPTY).is_some());
    assert!(expected_arguments(ERROR).is_none());
    assert!(expected_arguments("nope").is_none());
}

#[test]
fn helpers_exact_and_one_of() {
    assert!(exact(
        &types(&["Integer", "String"]),
        &types(&["Integer", "String"])
    ));
    assert!(!exact(&types(&["Integer"]), &types(&["Integer", "String"])));
    assert!(!exact(&types(&["String"]), &types(&["Integer"])));
    assert!(exact_one_of(
        &types(&["String"]),
        &types(&["String", "Integer"])
    ));
    assert!(!exact_one_of(
        &types(&["Boolean"]),
        &types(&["String", "Integer"])
    ));
    assert!(!exact_one_of(
        &types(&["String", "Integer"]),
        &types(&["String"])
    ));
}

/// bug-175 F, repointed by plan-111-F.
///
/// This pinned `general::function_parts`, which split a rendered FUNC
/// signature at the top-level `") AS "` rather than the first one — so
/// `FUNC(FUNC(Integer, Integer) AS Integer) AS Integer` kept its
/// higher-order parameter intact. That function is deleted (its last caller
/// took a `ParameterType`), and the contract belongs to the one grammar now.
/// The cases are unchanged; only the owner is.
#[test]
fn nested_function_parameters_split_at_the_top_level_arrow() {
    let parts = |spelling: &str| match ParameterType::parse(spelling) {
        ParameterType::Func(params, returns, false) => Some((
            params
                .iter()
                .map(|p| p.name().into_owned())
                .collect::<Vec<_>>(),
            returns.name().into_owned(),
        )),
        _ => None,
    };
    assert_eq!(
        parts("FUNC(Integer, String) AS Boolean"),
        Some((
            vec!["Integer".to_string(), "String".to_string()],
            "Boolean".to_string()
        ))
    );
    assert_eq!(
        parts("FUNC() AS Nothing"),
        Some((vec![], "Nothing".to_string()))
    );
    assert_eq!(parts("Integer"), None);
    assert_eq!(parts("FUNC(Integer)"), None);
    assert_eq!(
        parts("FUNC(FUNC(Integer, Integer) AS Integer) AS Integer"),
        Some((
            vec!["FUNC(Integer, Integer) AS Integer".to_string()],
            "Integer".to_string()
        ))
    );
    assert_eq!(
        parts("FUNC(String, FUNC(Integer, Integer) AS Integer) AS Boolean"),
        Some((
            vec![
                "String".to_string(),
                "FUNC(Integer, Integer) AS Integer".to_string()
            ],
            "Boolean".to_string()
        ))
    );
    assert_eq!(
        parts("FUNC(Integer) AS FUNC(Integer) AS Integer"),
        Some((
            vec!["Integer".to_string()],
            "FUNC(Integer) AS Integer".to_string()
        ))
    );
    assert_eq!(parts("FUNC(FUNC(Integer) AS Integer"), None);
}

/// The package registers exactly the 18 unqualified globals, all `Body::Intrinsic`,
/// and reproduces the legacy arities: `(1, 1)` for the single-argument members,
/// `(1, 2)` for the optional-tail `toString`/`toInt`, and `None` for the reserved
/// `error` (validated by `resolve_call`, not the arity gate).
#[test]
fn registers_the_eighteen_globals_with_legacy_arities() {
    let mut r = Registry::new();
    register(&mut r);
    let pkg = r.resolve_package("general").expect("general registered");
    assert!(pkg.is_unqualified_global());
    assert_eq!(pkg.functions().len(), ALL_GENERAL.len());

    for &name in ALL_GENERAL {
        let func = pkg
            .function(name)
            .unwrap_or_else(|| panic!("`{name}` missing"));
        let imp = func.implementations().first().expect("one implementation");
        assert!(
            matches!(imp.body, Body::Intrinsic),
            "{name} is Body::Intrinsic"
        );
    }

    // The package injects no source (all Intrinsic, no records/types), so the
    // generic `augment_project` pass skips it.
    assert!(pkg.get_mfb().is_empty());
}

/// The conversions carry the exact runtime errors the codegen contract
/// (`raise_error`) validates against, addressed by their `general.<name>` key.
#[test]
fn conversions_declare_their_runtime_errors() {
    let mut r = Registry::new();
    register(&mut r);
    assert!(r.declares_error("general.toFloat", "ErrOverflow"));
    assert!(r.declares_error("general.toFloat", "ErrInvalidFormat"));
    assert!(r.declares_error("general.toFixed", "ErrOverflow"));
    assert!(r.declares_error("general.toFixed", "ErrInvalidFormat"));
    assert!(r.declares_error("general.toByte", "ErrOverflow"));
    assert!(r.declares_error("general.toMoney", "ErrOverflow"));
    assert!(r.declares_error("general.toMoney", "ErrInvalidFormat"));
    assert!(r.declares_error("general.toScalar", "ErrInvalidArgument"));
    // bug-807: toString/toInt/error declared nothing while raising.
    assert!(r.declares_error("general.toString", "ErrEncoding"));
    assert!(r.declares_error("general.toString", "ErrOutOfMemory"));
    assert!(r.declares_error("general.toInt", "ErrInvalidFormat"));
    assert!(r.declares_error("general.toInt", "ErrOverflow"));
    assert!(r.declares_error("general.error", "ErrOutOfMemory"));
    // A conversion does not declare an error it never raises.
    assert!(!r.declares_error("general.toByte", "ErrInvalidFormat"));
    assert!(!r.declares_error("general.len", "ErrOverflow"));
}

/// `error`'s registry arity is a proper `(2, 2)` implementation, but the boundary
/// helper reproduces the legacy `None` so `error(x)` reports an argument mismatch
/// (via `resolve_call`), not an arity mismatch.
#[test]
fn error_arity_is_none_others_delegate() {
    assert_eq!(arity(ERROR), None);
    assert_eq!(arity(LEN), Some((1, 1)));
    assert_eq!(arity(TO_STRING), Some((1, 2)));
    assert_eq!(arity(TO_INT), Some((1, 2)));
    assert_eq!(arity(IS_NOT_EMPTY), Some((1, 1)));
}
