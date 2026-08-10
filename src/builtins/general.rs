use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, DefaultResolver,
    DefaultValue, Implementation, Lowering, Parameter, ParameterType, ReturnType,
};

const ERROR: &str = "error";
const LEN: &str = "len";
const TYPE_NAME: &str = "typeName";
const TO_STRING: &str = "toString";
const TO_INT: &str = "toInt";
const TO_FLOAT: &str = "toFloat";
const TO_FIXED: &str = "toFixed";
const TO_BYTE: &str = "toByte";
const TO_MONEY: &str = "toMoney";
const TO_SCALAR: &str = "toScalar";
const IS_NUMERIC: &str = "isNumeric";
const IS_EVEN: &str = "isEven";
const IS_ODD: &str = "isOdd";
const IS_POSITIVE: &str = "isPositive";
const IS_NEGATIVE: &str = "isNegative";
const IS_ZERO: &str = "isZero";
const IS_EMPTY: &str = "isEmpty";
const IS_NOT_EMPTY: &str = "isNotEmpty";

pub(crate) const BUILTIN_FUNCTION_ID_BASE: u32 = 0x8000_0000;
pub(crate) const BUILTIN_FUNCTION_IS_EVEN: u32 = BUILTIN_FUNCTION_ID_BASE + 1;
pub(crate) const BUILTIN_FUNCTION_IS_ODD: u32 = BUILTIN_FUNCTION_ID_BASE + 2;
pub(crate) const BUILTIN_FUNCTION_IS_POSITIVE: u32 = BUILTIN_FUNCTION_ID_BASE + 3;
pub(crate) const BUILTIN_FUNCTION_IS_NEGATIVE: u32 = BUILTIN_FUNCTION_ID_BASE + 4;
pub(crate) const BUILTIN_FUNCTION_IS_ZERO: u32 = BUILTIN_FUNCTION_ID_BASE + 5;
pub(crate) const BUILTIN_FUNCTION_IS_EMPTY: u32 = BUILTIN_FUNCTION_ID_BASE + 6;
pub(crate) const BUILTIN_FUNCTION_IS_NOT_EMPTY: u32 = BUILTIN_FUNCTION_ID_BASE + 7;
pub(crate) const BUILTIN_FUNCTION_IS_POSITIVE_FLOAT: u32 = BUILTIN_FUNCTION_ID_BASE + 8;
pub(crate) const BUILTIN_FUNCTION_IS_POSITIVE_FIXED: u32 = BUILTIN_FUNCTION_ID_BASE + 9;
pub(crate) const BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT: u32 = BUILTIN_FUNCTION_ID_BASE + 10;
pub(crate) const BUILTIN_FUNCTION_IS_NEGATIVE_FIXED: u32 = BUILTIN_FUNCTION_ID_BASE + 11;
pub(crate) const BUILTIN_FUNCTION_IS_ZERO_FLOAT: u32 = BUILTIN_FUNCTION_ID_BASE + 12;
pub(crate) const BUILTIN_FUNCTION_IS_ZERO_FIXED: u32 = BUILTIN_FUNCTION_ID_BASE + 13;
/// `isNumeric` was missing from `builtin_function_id` while the other seven
/// predicates were present, which is why it alone failed even in `filter` —
/// the one position that worked for the rest (bug-368).
pub(crate) const BUILTIN_FUNCTION_IS_NUMERIC: u32 = BUILTIN_FUNCTION_ID_BASE + 14;

// plan-72-L: `GENERAL` is the descriptor authority for the global (unqualified)
// builtins. Every function has a fixed return regardless of which accepted
// argument-type set matches, but the legacy `call_return_type_name` fast-path
// oracle populates ONLY the six numeric narrowing conversions (`toInt`..`toScalar`)
// and returns `None` for the rest — so those six carry `ReturnType::Fixed` and
// every other function carries `ReturnType::Custom` (→ `None`), reproducing the
// oracle exactly. `error` is an irregular reserved primitive: it is a member with
// parameter names but a `None` arity (its argument count is validated by
// `resolve_call`, not the generic arity gate), so its descriptor entry carries an
// EMPTY overload list — membership holds and `arity` is `None`, matching legacy.
// Its parameter names (`code`/`message`) therefore live only in the hand-authored
// `call_param_names` static until plan-72-BB (see Corrections in plan-72-L).
//
// `call_param_names`, `resolve_call`, and `expected_arguments` stay hand-authored:
// `call_param_names` returns a `&'static` borrowed shape the owned `DefaultResolver`
// cannot produce (and covers `error`); `resolve_call` performs per-position
// accepted-type-SET matching (`len` accepts String/List/Map/Set) the descriptor's
// single `ParameterType::Named` cannot express — so the parameter *types* below are
// illustrative, resolution is owned by `resolve_call`; `expected_arguments` uses a
// bespoke `"… or …"` phrasing. Each is pinned to `GENERAL` by
// `parity_matches_descriptor` where derivable.
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

// A fixed-return function whose return is not exposed through the context-free
// `call_return_type_name` oracle (resolved via `resolve_call` instead).
const fn ovc(params: &'static [Parameter]) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Custom,
    }
}

/// plan-88: a general builtin that declares the `errorCode` names it can raise at
/// runtime (the codegen contract validated by `raise_error`). Reuses `gfn`.
const fn gfn_err(
    name: &'static str,
    slug: &'static str,
    errors: &'static [&'static str],
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    let mut function = gfn(name, slug, overloads);
    function.errors = errors;
    function
}

const fn gfn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_into: "",
        doc_desc: "",
        errors: &[],
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

// Illustrative per-position types (see the note above). `value` is the canonical
// first-parameter name for every general call.
const P_V_STR: &[Parameter] = &[Parameter::required("value", "String")];
const P_V_INT: &[Parameter] = &[Parameter::required("value", "Integer")];
const P_V_T: &[Parameter] = &[Parameter::required("value", "T")];
const P_TO_STRING: &[Parameter] = &[
    Parameter::required("value", "Scalar"),
    Parameter {
        name: "precision",
        aliases: &["decimals"],
        ty: ParameterType::Named("Byte"),
        default: DefaultValue::Optional,
    },
];
const P_TO_INT: &[Parameter] = &[
    Parameter {
        name: "value",
        aliases: &["text"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
    Parameter {
        name: "base",
        aliases: &[],
        ty: ParameterType::Named("Integer"),
        default: DefaultValue::Optional,
    },
];

const GENERAL_FUNCTIONS: &[BuiltinFunction] = &[
    // Reserved primitive: member with param-names but None arity → empty overloads.
    gfn(ERROR, "error", &[]),
    gfn(LEN, "len", &[ovc(P_V_STR)]),
    gfn(TYPE_NAME, "typeName", &[ovc(P_V_T)]),
    gfn(TO_STRING, "toString", &[ovc(P_TO_STRING)]),
    gfn(TO_INT, "toInt", &[ov(P_TO_INT, "Integer")]),
    gfn_err(TO_FLOAT, "toFloat", &["ErrOverflow", "ErrInvalidFormat"], &[ov(P_V_STR, "Float")]),
    gfn_err(TO_FIXED, "toFixed", &["ErrOverflow", "ErrInvalidFormat"], &[ov(P_V_STR, "Fixed")]),
    gfn_err(TO_BYTE, "toByte", &["ErrOverflow"], &[ov(P_V_INT, "Byte")]),
    gfn_err(TO_MONEY, "toMoney", &["ErrInvalidFormat"], &[ov(P_V_STR, "Money")]),
    gfn_err(TO_SCALAR, "toScalar", &["ErrInvalidArgument"], &[ov(P_V_INT, "Scalar")]),
    gfn(IS_NUMERIC, "isNumeric", &[ovc(P_V_STR)]),
    gfn(IS_EVEN, "isEven", &[ovc(P_V_INT)]),
    gfn(IS_ODD, "isOdd", &[ovc(P_V_INT)]),
    gfn(IS_POSITIVE, "isPositive", &[ovc(P_V_INT)]),
    gfn(IS_NEGATIVE, "isNegative", &[ovc(P_V_INT)]),
    gfn(IS_ZERO, "isZero", &[ovc(P_V_INT)]),
    gfn(IS_EMPTY, "isEmpty", &[ovc(P_V_STR)]),
    gfn(IS_NOT_EMPTY, "isNotEmpty", &[ovc(P_V_STR)]),
];

/// Return-type resolution for the general calls, delegating to the hand-authored
/// `resolve_call` (the returns are argument-dependent — `len`, `error`, and the
/// generic overloads compute from operand types). Exposed through the descriptor
/// so plan-72-BB can drive `general::` return types from the registry.
struct GeneralResolver;
impl BuiltinResolver for GeneralResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        resolve_call(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static GENERAL_RESOLVER: GeneralResolver = GeneralResolver;

pub(crate) static GENERAL: BuiltinModule = BuiltinModule {
    name: "general",
    functions: GENERAL_FUNCTIONS,
    types: &[],
    source: None,
    resolver: Some(&GENERAL_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_general_call(name: &str) -> bool {
    DefaultResolver::contains(&GENERAL, name)
}

/// Whether a general built-in may be **overridden** by a user- or package-defined
/// `FUNC` of the same name for its own value types (plan-01-overload.md §A.2). Every
/// general call is overridable except `error`, which builds the read-only `Error`
/// record and is a reserved language primitive.
pub(crate) fn is_overridable(name: &str) -> bool {
    is_general_call(name) && name != ERROR
}

/// Whether a general built-in name is **reserved** and may not be declared as a
/// user `FUNC`/`SUB` (plan-01-overload.md §A.5). The reserved set is exactly
/// `{ error }`.
pub(crate) fn reserved_builtin_name(name: &str) -> bool {
    name == ERROR
}

/// The built-in's conventional result type for an overridable general call
/// (plan-01-overload.md §C, Phase 4). A **package-provided** override (routed
/// through the override registry) yields this declared result; a user override
/// yields its own declared return type instead. Returns `None` for `error` and any
/// non-general name.
pub(crate) fn override_result_type(name: &str) -> Option<&'static str> {
    match name {
        TO_STRING | TYPE_NAME => Some("String"),
        LEN | TO_INT => Some("Integer"),
        TO_FLOAT => Some("Float"),
        TO_FIXED => Some("Fixed"),
        TO_BYTE => Some("Byte"),
        TO_MONEY => Some("Money"),
        TO_SCALAR => Some("Scalar"),
        IS_NUMERIC | IS_EVEN | IS_ODD | IS_POSITIVE | IS_NEGATIVE | IS_ZERO | IS_EMPTY
        | IS_NOT_EMPTY => Some("Boolean"),
        _ => None,
    }
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        ERROR => Some(&[&["code"], &["message"]]),
        LEN => Some(&[&["value"]]),
        TYPE_NAME => Some(&[&["value"]]),
        TO_STRING => Some(&[&["value"], &["precision", "decimals"]]),
        TO_INT => Some(&[&["value", "text"], &["base"]]),
        TO_FLOAT => Some(&[&["value"]]),
        TO_FIXED => Some(&[&["value"]]),
        TO_BYTE => Some(&[&["value"]]),
        TO_MONEY => Some(&[&["value"]]),
        TO_SCALAR => Some(&[&["value"]]),
        IS_NUMERIC => Some(&[&["value"]]),
        IS_EVEN => Some(&[&["value"]]),
        IS_ODD => Some(&[&["value"]]),
        IS_POSITIVE => Some(&[&["value"]]),
        IS_NEGATIVE => Some(&[&["value"]]),
        IS_ZERO => Some(&[&["value"]]),
        IS_EMPTY => Some(&[&["value"]]),
        IS_NOT_EMPTY => Some(&[&["value"]]),
        _ => None,
    }
}

pub(crate) fn builtin_function_id(name: &str) -> Option<u32> {
    match name {
        IS_EVEN => Some(BUILTIN_FUNCTION_IS_EVEN),
        IS_ODD => Some(BUILTIN_FUNCTION_IS_ODD),
        IS_POSITIVE => Some(BUILTIN_FUNCTION_IS_POSITIVE),
        IS_NEGATIVE => Some(BUILTIN_FUNCTION_IS_NEGATIVE),
        IS_ZERO => Some(BUILTIN_FUNCTION_IS_ZERO),
        IS_EMPTY => Some(BUILTIN_FUNCTION_IS_EMPTY),
        IS_NOT_EMPTY => Some(BUILTIN_FUNCTION_IS_NOT_EMPTY),
        IS_NUMERIC => Some(BUILTIN_FUNCTION_IS_NUMERIC),
        _ => None,
    }
}

pub(crate) fn builtin_function_id_for_type(name: &str, function_type: &str) -> Option<u32> {
    let (params, returns) = function_parts(function_type)?;
    if params.len() != 1 || returns != "Boolean" {
        return builtin_function_id(name);
    }
    match (name, params[0]) {
        (IS_POSITIVE, "Float") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT),
        (IS_POSITIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED),
        (IS_NEGATIVE, "Float") => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT),
        (IS_NEGATIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED),
        (IS_ZERO, "Float") => Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT),
        (IS_ZERO, "Fixed") => Some(BUILTIN_FUNCTION_IS_ZERO_FIXED),
        _ => builtin_function_id(name),
    }
}

pub(crate) fn filter_predicate_type(name: &str, element_type: &str) -> Option<String> {
    builtin_function_id(name)?;
    let arg_types = vec![element_type.to_string()];
    let resolved = resolve_call(name, &arg_types)?;
    (resolved.return_type == "Boolean").then(|| format!("FUNC({element_type}) AS Boolean"))
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let resolved = match name {
        ERROR => {
            if exact(arg_types, &["Integer", "String"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Error"),
                }
            } else {
                return None;
            }
        }
        LEN => {
            if arg_types.len() != 1 {
                return None;
            }
            if arg_types[0] == "String"
                || arg_types[0].starts_with("List OF ")
                || arg_types[0].starts_with("Map OF ")
                || arg_types[0].starts_with("Set OF ")
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("Integer"),
                }
            } else {
                return None;
            }
        }
        TYPE_NAME => {
            if arg_types.len() == 1 {
                ResolvedCall {
                    return_type: Cow::Borrowed("String"),
                }
            } else {
                return None;
            }
        }
        TO_STRING => {
            if arg_types.len() == 2
                && matches!(arg_types[0].as_str(), "Float" | "Fixed" | "Money")
                && arg_types[1] == "Byte"
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("String"),
                }
            } else if arg_types.len() == 1
                && (matches!(
                    arg_types[0].as_str(),
                    "Integer"
                        | "Float"
                        | "Fixed"
                        | "Money"
                        | "Boolean"
                        | "String"
                        | "Byte"
                        | "Scalar"
                ) || arg_types[0] == "List OF Byte")
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("String"),
                }
            } else {
                return None;
            }
        }
        TO_INT => {
            // 1-arg: parse base-10 (String) or numeric narrowing (Byte/Float/Fixed).
            // 2-arg: `toInt(text AS String, base AS Integer)` parses `text` in
            // `base` (plan-02-cleanup §5). The optional `base` is a second arity,
            // not a user-level default parameter, since `toInt` is overloaded.
            if exact_one_of(
                arg_types,
                &["String", "Byte", "Float", "Fixed", "Money", "Scalar"],
            ) || exact(arg_types, &["String", "Integer"])
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("Integer"),
                }
            } else {
                return None;
            }
        }
        TO_FLOAT => {
            if exact_one_of(arg_types, &["String", "Integer", "Fixed", "Money"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Float"),
                }
            } else {
                return None;
            }
        }
        TO_FIXED => {
            if exact_one_of(arg_types, &["String", "Integer", "Float", "Money"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Fixed"),
                }
            } else {
                return None;
            }
        }
        TO_BYTE => {
            if exact_one_of(arg_types, &["Integer", "Money", "Scalar"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Byte"),
                }
            } else {
                return None;
            }
        }
        TO_SCALAR => {
            // Narrowing into a codepoint. `toScalar(Byte)` is infallible (every
            // byte 0..255 is a valid non-surrogate scalar); `toScalar(Integer)`
            // and `toScalar(String)` are fallible (surrogate/range or non-single-
            // scalar string trap `ErrInvalidArgument`) (plan-41-D §1).
            if exact_one_of(arg_types, &["Integer", "String", "Byte"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Scalar"),
                }
            } else {
                return None;
            }
        }
        TO_MONEY => {
            // Explicit crossing into Money from every scalar type (plan-29-G §4.2).
            if exact_one_of(arg_types, &["String", "Integer", "Float", "Fixed", "Byte"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Money"),
                }
            } else {
                return None;
            }
        }
        IS_NUMERIC => {
            if exact(arg_types, &["String"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
                }
            } else {
                return None;
            }
        }
        IS_EVEN | IS_ODD => {
            if exact(arg_types, &["Integer"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
                }
            } else {
                return None;
            }
        }
        IS_POSITIVE | IS_NEGATIVE | IS_ZERO => {
            if exact_one_of(arg_types, &["Integer", "Float", "Fixed"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
                }
            } else {
                return None;
            }
        }
        IS_EMPTY | IS_NOT_EMPTY
            if arg_types.len() == 1
                && (arg_types[0] == "String"
                    || arg_types[0].starts_with("List OF ")
                    || arg_types[0].starts_with("Map OF ")
                    || arg_types[0].starts_with("Set OF ")) =>
        {
            ResolvedCall {
                return_type: Cow::Borrowed("Boolean"),
            }
        }
        _ => return None,
    };
    Some(resolved)
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        LEN => Some("String, List OF T, Set OF T, or Map OF K TO V"),
        TYPE_NAME => Some("T"),
        TO_STRING => Some(
            "Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, or List OF Byte",
        ),
        TO_INT => Some("String[, Integer], Byte, Float, Fixed, Money, or Scalar"),
        TO_FLOAT => Some("String, Integer, Fixed, or Money"),
        TO_FIXED => Some("String, Integer, Float, or Money"),
        TO_BYTE => Some("Integer, Money, or Scalar"),
        TO_MONEY => Some("String, Integer, Float, Fixed, or Byte"),
        TO_SCALAR => Some("Integer, String, or Byte"),
        IS_NUMERIC => Some("String"),
        IS_EVEN => Some("Integer"),
        IS_ODD => Some("Integer"),
        IS_POSITIVE | IS_NEGATIVE | IS_ZERO => Some("Integer, Float, or Fixed"),
        IS_EMPTY | IS_NOT_EMPTY => Some("String, List OF T, Set OF T, or Map OF K TO V"),
        _ => None,
    }
}

use super::exact;

fn exact_one_of(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == 1 && expected.iter().any(|expected| arg_types[0] == *expected)
}
/// The element type of a `List`, with any `RES` ownership-axis marker stripped:
/// a `List OF RES File` yields the pointer element type `File`, since reading or
/// inserting an element works with the bare resource value (§15.6).
pub(super) fn list_element(type_name: &str) -> Option<&str> {
    let element = type_name.strip_prefix("List OF ")?;
    Some(element.strip_prefix("RES ").unwrap_or(element))
}

pub(super) fn map_parts(type_name: &str) -> Option<(&str, &str)> {
    let (key, value) = type_name.strip_prefix("Map OF ")?.split_once(" TO ")?;
    Some((key, value.strip_prefix("RES ").unwrap_or(value)))
}

/// The element type of a `Set OF T` (plan-63). A Set element is always
/// comparable and never `RES`-marked, so there is no marker to strip.
pub(super) fn set_element(type_name: &str) -> Option<&str> {
    type_name.strip_prefix("Set OF ")
}

/// Splits a `FUNC(<params>) AS <return>` type into its parameter types and its
/// return type.
///
/// A parameter can itself be a function type — `FUNC(FUNC(Integer, Integer) AS
/// Integer) AS Integer` is what `collections::transform` receives over a list of
/// two-argument function values — so the parameter list is scanned with paren
/// depth: the closing paren and the separating commas are the ones at depth 0.
pub(super) fn function_parts(type_name: &str) -> Option<(Vec<&str>, &str)> {
    super::split_func_params_and_return(type_name.strip_prefix("FUNC(")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
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
        assert!(!is_general_call("nope"));
        assert!(!is_general_call("collections.get"));
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
        assert_eq!(builtin_function_id(LEN), None);
        assert_eq!(builtin_function_id("nope"), None);
    }

    #[test]
    fn builtin_function_id_for_type_specialized() {
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Float) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Fixed) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_NEGATIVE, "FUNC(Float) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_NEGATIVE, "FUNC(Fixed) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_ZERO, "FUNC(Float) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_ZERO, "FUNC(Fixed) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_ZERO_FIXED)
        );
        // Integer element falls through to the plain id.
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE)
        );
        // Non-predicate specialization name (isEven) -> plain id.
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
    }

    #[test]
    fn builtin_function_id_for_type_non_predicate_shape() {
        // Not a single-param Boolean predicate -> falls back to builtin_function_id.
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer, Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer) AS Integer"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
        // Not a FUNC type at all -> None from function_parts.
        assert_eq!(builtin_function_id_for_type(IS_EVEN, "Integer"), None);
    }

    #[test]
    fn filter_predicate_type_cases() {
        assert_eq!(
            filter_predicate_type(IS_EVEN, "Integer"),
            Some("FUNC(Integer) AS Boolean".to_string())
        );
        assert_eq!(
            filter_predicate_type(IS_POSITIVE, "Float"),
            Some("FUNC(Float) AS Boolean".to_string())
        );
        // Not a builtin_function_id name -> None.
        assert_eq!(filter_predicate_type(LEN, "String"), None);
        // Element type the predicate does not resolve for -> None.
        assert_eq!(filter_predicate_type(IS_EVEN, "String"), None);
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
        assert_eq!(rt(TO_STRING, &["Integer", "Byte"]), None); // Integer has no 2-arg form
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
    fn expected_arguments_all_arms() {
        assert!(expected_arguments(LEN).is_some());
        assert!(expected_arguments(TYPE_NAME).is_some());
        assert!(expected_arguments(TO_STRING).is_some());
        assert!(expected_arguments(TO_INT).is_some());
        assert!(expected_arguments(TO_FLOAT).is_some());
        assert!(expected_arguments(TO_FIXED).is_some());
        assert!(expected_arguments(TO_BYTE).is_some());
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
            &strings(&["Integer", "String"]),
            &["Integer", "String"]
        ));
        assert!(!exact(&strings(&["Integer"]), &["Integer", "String"]));
        assert!(!exact(&strings(&["String"]), &["Integer"]));
        assert!(exact_one_of(&strings(&["String"]), &["String", "Integer"]));
        assert!(!exact_one_of(
            &strings(&["Boolean"]),
            &["String", "Integer"]
        ));
        assert!(!exact_one_of(&strings(&["String", "Integer"]), &["String"]));
    }

    #[test]
    fn helpers_list_map_function_parts() {
        assert_eq!(list_element("List OF Integer"), Some("Integer"));
        assert_eq!(list_element("List OF RES File"), Some("File"));
        assert_eq!(list_element("Integer"), None);
        assert_eq!(
            map_parts("Map OF String TO Integer"),
            Some(("String", "Integer"))
        );
        assert_eq!(
            map_parts("Map OF String TO RES File"),
            Some(("String", "File"))
        );
        assert_eq!(map_parts("Integer"), None);
        assert_eq!(map_parts("Map OF String"), None);
        assert_eq!(
            function_parts("FUNC(Integer, String) AS Boolean"),
            Some((vec!["Integer", "String"], "Boolean"))
        );
        assert_eq!(
            function_parts("FUNC() AS Nothing"),
            Some((vec![], "Nothing"))
        );
        assert_eq!(function_parts("Integer"), None);
        assert_eq!(function_parts("FUNC(Integer)"), None);
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov` builds a Fixed-return overload; `ovc` a Custom-return one. Both are
        // only invoked in const context by GENERAL_FUNCTIONS, so call them here.
        let fixed = ov(P_V_INT, "Byte");
        assert_eq!(fixed.params.len(), 1);
        assert_eq!(fixed.params[0].name, "value");
        assert_eq!(fixed.return_type, ReturnType::Fixed("Byte"));

        let custom = ovc(P_V_STR);
        assert_eq!(custom.params.len(), 1);
        assert_eq!(custom.params[0].name, "value");
        assert_eq!(custom.return_type, ReturnType::Custom);

        // `gfn` assembles a general BuiltinFunction (Same/Helper, no flags).
        // E0716: gfn wants a &'static overload slice, so bind a const first.
        const OV: &[BuiltinOverload] = &[ov(P_V_INT, "Byte")];
        let func = gfn("demoName", "demoSlug", OV);
        assert_eq!(func.name, "demoName");
        assert_eq!(func.doc_slug, "demoSlug");
        assert_eq!(func.overloads.len(), 1);
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Helper);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }

    #[test]
    fn override_result_type_money_scalar_arms() {
        assert_eq!(override_result_type(TO_MONEY), Some("Money"));
        assert_eq!(override_result_type(TO_SCALAR), Some("Scalar"));
    }

    #[test]
    fn call_param_names_money_scalar_arms() {
        assert_eq!(call_param_names(TO_MONEY), Some(&[&["value"][..]][..]));
        assert_eq!(call_param_names(TO_SCALAR), Some(&[&["value"][..]][..]));
    }

    #[test]
    fn builtin_function_id_is_numeric_arm() {
        assert_eq!(
            builtin_function_id(IS_NUMERIC),
            Some(BUILTIN_FUNCTION_IS_NUMERIC)
        );
    }

    #[test]
    fn resolve_to_scalar_and_money() {
        // toScalar accepts Integer/String/Byte -> Scalar, rejects others.
        assert_eq!(rt(TO_SCALAR, &["Integer"]), Some("Scalar".to_string()));
        assert_eq!(rt(TO_SCALAR, &["String"]), Some("Scalar".to_string()));
        assert_eq!(rt(TO_SCALAR, &["Byte"]), Some("Scalar".to_string()));
        assert_eq!(rt(TO_SCALAR, &["Float"]), None);
        assert_eq!(rt(TO_SCALAR, &["Integer", "Integer"]), None);
        // toMoney accepts String/Integer/Float/Fixed/Byte -> Money.
        assert_eq!(rt(TO_MONEY, &["String"]), Some("Money".to_string()));
        assert_eq!(rt(TO_MONEY, &["Integer"]), Some("Money".to_string()));
        assert_eq!(rt(TO_MONEY, &["Float"]), Some("Money".to_string()));
        assert_eq!(rt(TO_MONEY, &["Fixed"]), Some("Money".to_string()));
        assert_eq!(rt(TO_MONEY, &["Byte"]), Some("Money".to_string()));
        assert_eq!(rt(TO_MONEY, &["Boolean"]), None);
    }

    #[test]
    fn expected_arguments_money_scalar_arms() {
        assert_eq!(
            expected_arguments(TO_MONEY),
            Some("String, Integer, Float, Fixed, or Byte")
        );
        assert_eq!(expected_arguments(TO_SCALAR), Some("Integer, String, or Byte"));
    }

    #[test]
    fn function_parts_splits_nested_function_parameters() {
        // A flat `split_once(") AS ")` cut at the *inner* `) AS `, yielding the
        // garbage params ["FUNC(Integer", "Integer"] and return "Integer) AS X".
        assert_eq!(
            function_parts("FUNC(FUNC(Integer, Integer) AS Integer) AS Integer"),
            Some((vec!["FUNC(Integer, Integer) AS Integer"], "Integer"))
        );
        assert_eq!(
            function_parts("FUNC(String, FUNC(Integer, Integer) AS Integer) AS Boolean"),
            Some((
                vec!["String", "FUNC(Integer, Integer) AS Integer"],
                "Boolean"
            ))
        );
        // The return type may itself be a function type.
        assert_eq!(
            function_parts("FUNC(Integer) AS FUNC(Integer) AS Integer"),
            Some((vec!["Integer"], "FUNC(Integer) AS Integer"))
        );
        // An unbalanced parameter list has no top-level close paren.
        assert_eq!(function_parts("FUNC(FUNC(Integer) AS Integer"), None);
    }

}
