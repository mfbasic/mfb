//! The built-in `general` package — the **unqualified global** builtins.
//!
//! These are the bare-name builtins available in every program without an `IMPORT`:
//! `error`, `len`, `typeName`, the numeric conversions (`toString`/`toInt`/`toFloat`/
//! `toFixed`/`toByte`/`toMoney`/`toScalar`), and the predicates (`isNumeric`/`isEven`/
//! …/`isNotEmpty`). They are written as bare names (`len(xs)`), never
//! `general::len`, and there is no writable `IMPORT general`.
//!
//! Like [`crate::codegen::builtins::testing`], the package is registered under the
//! real name `"general"` only so it has a home in the registry: because the registry's
//! qualified query surface (`resolve_func` / `owning_package` / `arity` /
//! `declares_error`) all require a `.` (`split_once('.')`), a bare `toFloat` is inert
//! to those, so the real package name costs nothing at call sites. The runtime layer
//! already names this family `"general"` (`RuntimeHelper::General`, `_mfb_rt_general_*`),
//! so the name is not arbitrary. The package carries the additive
//! [`RegistryPackage::mark_unqualified_global`] flag so `mfb man2 --all` skips it.
//!
//! WHY the hand-authored [`resolve_call`] rather than the generic matcher: the
//! argument-dependent returns need per-position accepted-type-SET matching the generic
//! matcher cannot express (`len`/`isEmpty`/`isNotEmpty` accept String OR List/Map/Set;
//! `toString` accepts nine scalars plus `List OF Byte`; `error` accepts exactly
//! `[Integer, String]`). So each registered [`Implementation`] carries only illustrative
//! parameter *types* and a [`Body::Intrinsic`] marker — resolution is owned by
//! `resolve_call`, and codegen is the existing `RuntimeHelper::General` bare-name
//! lowering (`builder_conversions`/`builder_values`/`module_analysis`).
//!
//! The registry entries exist PRIMARILY so `declares_error("general.toX", err)`
//! answers for the conversion codegen contract (`raise_error`) and so `arity` matches
//! legacy. `error` is an irregular reserved primitive: legacy `error` had param names
//! but EMPTY overloads → `None` arity (its argument count is validated by `resolve_call`,
//! not the generic arity gate). The clean-room registry forbids an implementation-less
//! function, so `error` carries one illustrative implementation but [`arity`] special-
//! cases it back to `None`, reproducing the legacy diagnostic exactly.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, Registry, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
use std::borrow::Cow;

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

// ---------------------------------------------------------------------------
// Membership & classification (bare-name — the calls never carry a `.`).
// ---------------------------------------------------------------------------

/// Whether `name` is one of the eighteen unqualified global builtins. A bare
/// `matches!` table over the member names (the calls stay unqualified end-to-end, so
/// the registry's `.`-requiring query surface never sees them).
pub(crate) fn is_general_call(name: &str) -> bool {
    matches!(
        name,
        ERROR
            | LEN
            | TYPE_NAME
            | TO_STRING
            | TO_INT
            | TO_FLOAT
            | TO_FIXED
            | TO_BYTE
            | TO_MONEY
            | TO_SCALAR
            | IS_NUMERIC
            | IS_EVEN
            | IS_ODD
            | IS_POSITIVE
            | IS_NEGATIVE
            | IS_ZERO
            | IS_EMPTY
            | IS_NOT_EMPTY
    )
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
    match (name, params[0].as_str()) {
        (IS_POSITIVE, "Float") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT),
        (IS_POSITIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED),
        (IS_NEGATIVE, "Float") => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT),
        (IS_NEGATIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED),
        (IS_ZERO, "Float") => Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT),
        (IS_ZERO, "Fixed") => Some(BUILTIN_FUNCTION_IS_ZERO_FIXED),
        _ => builtin_function_id(name),
    }
}

pub(crate) fn filter_predicate_type(name: &str, element_type: &ParameterType) -> Option<String> {
    builtin_function_id(name)?;
    let arg_types = vec![element_type.to_string()];
    let resolved = resolve_call(name, &arg_types)?;
    (resolved.return_type == "Boolean").then(|| format!("FUNC({element_type}) AS Boolean"))
}

/// The typed twin of [`filter_predicate_type`] (plan-106-A): the callback type a
/// bare general builtin adopts when passed as a `filter`/`transform` predicate
/// over `element_type`, built as a [`Func`](crate::types::ParameterType::Func)
/// rather than `format!`ed.
///
/// The `resolve_call` half still speaks names (`general`'s bespoke resolver is one
/// of the three the typed registry entry does not cover — plan-104-C), so the
/// element renders for that lookup only.
pub(crate) fn filter_predicate_type_typed(
    name: &str,
    element_type: &crate::types::ParameterType,
) -> Option<crate::types::ParameterType> {
    use crate::types::ParameterType;
    builtin_function_id(name)?;
    let arg_types = vec![element_type.name().into_owned()];
    let resolved = resolve_call(name, &arg_types)?;
    (resolved.return_type == "Boolean").then(|| {
        ParameterType::Func(
            vec![element_type.clone()],
            Box::new(ParameterType::Boolean),
            false,
        )
    })
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

/// Splits a `FUNC(<params>) AS <return>` type into its parameter types and its
/// return type.
///
/// A parameter can itself be a function type — `FUNC(FUNC(Integer, Integer) AS
/// Integer) AS Integer` is what `collections::transform` receives over a list of
/// two-argument function values — so the parameter list is scanned with paren
/// depth: the closing paren and the separating commas are the ones at depth 0.
///
/// plan-106-E: the split is the [`ParameterType::Func`] variant's own fields, not
/// a `strip_prefix("FUNC(")` plus a depth scan. `ISOLATED FUNC(…)` still answers
/// `None`, as the bare-`FUNC(` strip did — the isolated flag is matched
/// explicitly rather than falling out of the prefix.
pub(crate) fn function_parts(type_name: &str) -> Option<(Vec<String>, String)> {
    match crate::types::ParameterType::parse(type_name) {
        crate::types::ParameterType::Func(params, returns, false) => Some((
            params.iter().map(|p| p.name().into_owned()).collect(),
            returns.name().into_owned(),
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Return-type resolution (argument-dependent — the bespoke resolver).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

/// Return-type resolution for the general calls, the successor of
/// `GeneralResolver::resolve_return_type` — delegates to the hand-authored
/// [`resolve_call`] (the returns are argument-dependent). The `strict` mode of the
/// generic registry oracle does not apply: general resolution has always been a single
/// per-position accepted-type-set match.
pub(crate) fn resolve_return_type(
    name: &str,
    arg_types: &[crate::types::ParameterType],
) -> Option<crate::types::ParameterType> {
    // plan-111-C: typed at the boundary. `general`'s own `resolve_call` matches
    // over a hand-authored signature table keyed by spelling, so the arguments
    // render for it and its answer is classified once, here, instead of by every
    // caller.
    let names: Vec<String> = arg_types.iter().map(|a| a.name().into_owned()).collect();
    resolve_call(name, &names)
        .map(|resolved| crate::types::ParameterType::parse(&resolved.return_type))
}

/// The static (argument-independent) nominal return of a general call — the six
/// numeric narrowing conversions carry a fixed return type; every other general call
/// resolved through `resolve_call` (`Custom`) yields `None`. Reproduces the legacy
/// `call_return_type_name` fast-oracle (`DefaultResolver::return_type_name` over the
/// `ReturnType::Fixed`/`Custom` split), consumed by `term_return_type`.
pub(crate) fn nominal_return_type(name: &str) -> Option<&'static str> {
    match name {
        TO_INT => Some("Integer"),
        TO_FLOAT => Some("Float"),
        TO_FIXED => Some("Fixed"),
        TO_BYTE => Some("Byte"),
        TO_MONEY => Some("Money"),
        TO_SCALAR => Some("Scalar"),
        _ => None,
    }
}

/// The `(min, max)` argument arity of a general call. The reserved primitive `error`
/// has `None` arity (legacy `error` had EMPTY overloads — its argument count is
/// validated by [`resolve_call`], not the generic arity gate), reproduced here as a
/// special case; every other member delegates to the registry's `general.<name>`
/// arity so the single source of truth is the registered `Implementation`.
pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    if name == ERROR {
        return None;
    }
    crate::codegen::registry::registry().arity(&format!("general.{name}"))
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
                || crate::codegen::engine::types::is_collection_type(&arg_types[0])
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
            // 2-arg `(Float|Fixed|Money, Byte)` precision form, or 1-arg over the nine
            // scalars plus `List OF Byte`. Both yield `String`.
            let two_arg = arg_types.len() == 2
                && matches!(arg_types[0].as_str(), "Float" | "Fixed" | "Money")
                && arg_types[1] == "Byte";
            let one_arg = arg_types.len() == 1
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
                        | "AttributedString"
                ) || arg_types[0] == "List OF Byte");
            if two_arg || one_arg {
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
                    || crate::codegen::engine::types::is_collection_type(&arg_types[0])) =>
        {
            ResolvedCall {
                return_type: Cow::Borrowed("Boolean"),
            }
        }
        _ => return None,
    };
    Some(resolved)
}

use crate::codegen::builtins::exact;
fn exact_one_of(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == 1 && expected.iter().any(|expected| arg_types[0] == *expected)
}

// ---------------------------------------------------------------------------
// Registration (membership / arity / declared-errors home in the clean-room
// registry). Each member carries illustrative parameter types and a
// `Body::Intrinsic` marker — resolution is `resolve_call`, codegen is the existing
// `RuntimeHelper::General` bare-name lowering.
// ---------------------------------------------------------------------------

/// A required parameter of illustrative type `ty`.
fn req(name: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::None,
    }
}

/// An optional parameter (widens arity but is never default-padded — the runtime
/// lowering selects the overload by argument count).
fn opt(name: &'static str, aliases: &'static [&'static str], ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases,
        ty,
        default: DefaultValue::Optional,
    }
}

/// Build one general member: a single `Body::Intrinsic` implementation carrying its
/// illustrative signature, return type, and declared runtime errors.
fn member(
    name: &'static str,
    intro: &'static str,
    return_type: ParameterType,
    errors: Vec<&'static str>,
    params: Vec<Parameter>,
) -> RegistryFunction {
    RegistryFunction {
        name,
        intro,
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params,
            return_type,
            errors,
            body: Body::Intrinsic,
        }],
    }
}

const INTRO: &str = "The always-available global builtins (no `IMPORT` required).";

const DESC: &str =
    "The `general` builtins are the unqualified global functions available in every \
program without an `IMPORT`: `error`, `len`, `typeName`, the numeric conversions \
(`toString`/`toInt`/`toFloat`/`toFixed`/`toByte`/`toMoney`/`toScalar`), and the \
predicates (`isNumeric`/`isEven`/…/`isNotEmpty`). They are written as bare names and \
have no `general::` spelling.";

/// Register the `general` package on the clean-room registry. See the module docs for
/// why it is a real-named-but-unqualified-global package and why `error` is irregular.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("general", INTRO, DESC);
    pkg.mark_unqualified_global();

    // Reserved primitive: legacy `error` had param names but EMPTY overloads → `None`
    // arity. The registry forbids an implementation-less function, so it carries one
    // illustrative implementation; `arity` special-cases `error` back to `None`.
    pkg.add_function(member(
        ERROR,
        "Construct an `Error` value from a numeric code and a message.",
        ParameterType::named("Error"),
        vec![],
        vec![
            req("code", ParameterType::Integer),
            req("message", ParameterType::String),
        ],
    ));
    pkg.add_function(member(
        LEN,
        "The number of elements in a String, List, Set, or Map.",
        ParameterType::Integer,
        vec![],
        vec![req("value", ParameterType::String)],
    ));
    pkg.add_function(member(
        TYPE_NAME,
        "The name of a value's runtime type.",
        ParameterType::String,
        vec![],
        vec![req("value", ParameterType::var("T"))],
    ));
    pkg.add_function(member(
        TO_STRING,
        "Render a value as a String.",
        ParameterType::String,
        vec![],
        vec![
            req("value", ParameterType::named("Scalar")),
            opt("precision", &["decimals"], ParameterType::Byte),
        ],
    ));
    pkg.add_function(member(
        TO_INT,
        "Convert a value to an Integer.",
        ParameterType::Integer,
        vec![],
        vec![
            req("value", ParameterType::String),
            opt("base", &[], ParameterType::Integer),
        ],
    ));
    pkg.add_function(member(
        TO_FLOAT,
        "Convert a value to a Float.",
        ParameterType::Float,
        vec!["ErrOverflow", "ErrInvalidFormat"],
        vec![req("value", ParameterType::String)],
    ));
    pkg.add_function(member(
        TO_FIXED,
        "Convert a value to a Fixed.",
        ParameterType::Fixed,
        vec!["ErrOverflow", "ErrInvalidFormat"],
        vec![req("value", ParameterType::String)],
    ));
    pkg.add_function(member(
        TO_BYTE,
        "Convert a value to a Byte.",
        ParameterType::Byte,
        vec!["ErrOverflow"],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        TO_MONEY,
        "Convert a value to Money.",
        ParameterType::Money,
        vec!["ErrOverflow", "ErrInvalidFormat"],
        vec![req("value", ParameterType::String)],
    ));
    pkg.add_function(member(
        TO_SCALAR,
        "Convert a value to a Scalar (Unicode codepoint).",
        ParameterType::named("Scalar"),
        vec!["ErrInvalidArgument"],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        IS_NUMERIC,
        "Whether a String parses as a number.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::String)],
    ));
    pkg.add_function(member(
        IS_EVEN,
        "Whether an Integer is even.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        IS_ODD,
        "Whether an Integer is odd.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        IS_POSITIVE,
        "Whether a number is greater than zero.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        IS_NEGATIVE,
        "Whether a number is less than zero.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        IS_ZERO,
        "Whether a number equals zero.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::Integer)],
    ));
    pkg.add_function(member(
        IS_EMPTY,
        "Whether a String, List, Set, or Map has no elements.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::String)],
    ));
    pkg.add_function(member(
        IS_NOT_EMPTY,
        "Whether a String, List, Set, or Map has at least one element.",
        ParameterType::Boolean,
        vec![],
        vec![req("value", ParameterType::String)],
    ));

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// plan-111-C: `resolve_return_type` takes and returns types now.
    fn types(items: &[&str]) -> Vec<ParameterType> {
        items.iter().map(|s| ParameterType::parse(s)).collect()
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
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
    }

    #[test]
    fn builtin_function_id_for_type_non_predicate_shape() {
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer, Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer) AS Integer"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
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
        assert_eq!(filter_predicate_type(LEN, "String"), None);
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
            resolve_return_type(TO_MONEY, &types(&["Integer"])),
            Some(ParameterType::Money)
        );
        assert_eq!(resolve_return_type("nope", &types(&["Integer"])), None);
    }

    #[test]
    fn nominal_return_type_matches_fast_oracle() {
        // The six numeric narrowing conversions carry a fixed return type.
        assert_eq!(nominal_return_type(TO_INT), Some("Integer"));
        assert_eq!(nominal_return_type(TO_FLOAT), Some("Float"));
        assert_eq!(nominal_return_type(TO_FIXED), Some("Fixed"));
        assert_eq!(nominal_return_type(TO_BYTE), Some("Byte"));
        assert_eq!(nominal_return_type(TO_MONEY), Some("Money"));
        assert_eq!(nominal_return_type(TO_SCALAR), Some("Scalar"));
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
    fn function_parts_splits_nested_function_parameters() {
        assert_eq!(
            function_parts("FUNC(Integer, String) AS Boolean"),
            Some((
                vec!["Integer".to_string(), "String".to_string()],
                "Boolean".to_string()
            ))
        );
        assert_eq!(
            function_parts("FUNC() AS Nothing"),
            Some((vec![], "Nothing".to_string()))
        );
        assert_eq!(function_parts("Integer"), None);
        assert_eq!(function_parts("FUNC(Integer)"), None);
        assert_eq!(
            function_parts("FUNC(FUNC(Integer, Integer) AS Integer) AS Integer"),
            Some((
                vec!["FUNC(Integer, Integer) AS Integer".to_string()],
                "Integer".to_string()
            ))
        );
        assert_eq!(
            function_parts("FUNC(String, FUNC(Integer, Integer) AS Integer) AS Boolean"),
            Some((
                vec![
                    "String".to_string(),
                    "FUNC(Integer, Integer) AS Integer".to_string()
                ],
                "Boolean".to_string()
            ))
        );
        assert_eq!(
            function_parts("FUNC(Integer) AS FUNC(Integer) AS Integer"),
            Some((
                vec!["Integer".to_string()],
                "FUNC(Integer) AS Integer".to_string()
            ))
        );
        assert_eq!(function_parts("FUNC(FUNC(Integer) AS Integer"), None);
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
}
