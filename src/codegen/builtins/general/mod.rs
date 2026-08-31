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

// One file per member, holding its descriptor AND its man-page prose — the same
// shape every other builtin package uses. The shared helpers (`req`, `opt`,
// `member`) and the name constants stay here; so does `resolve_call`, which owns
// resolution for the whole family.
mod func_error;
mod func_is_empty;
mod func_is_even;
mod func_is_negative;
mod func_is_not_empty;
mod func_is_numeric;
mod func_is_odd;
mod func_is_positive;
mod func_is_zero;
mod func_len;
mod func_to_byte;
mod func_to_fixed;
mod func_to_float;
mod func_to_int;
mod func_to_money;
mod func_to_scalar;
mod func_to_string;
mod func_type_name;

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

pub(crate) fn builtin_function_id_for_type(
    name: &str,
    function_type: &ParameterType,
) -> Option<u32> {
    // plan-111-F: the FUNC shape is the variant's own fields, so this reads the
    // single parameter off it instead of splitting a rendered signature.
    //
    // A non-FUNC type answers `None`, NOT `builtin_function_id(name)` — that is
    // what the `function_parts(...)?` this replaced did, and conflating the two
    // makes a bare `Integer` resolve to the unspecialized id (caught by
    // `builtin_function_id_for_type_non_predicate_shape`).
    let ParameterType::Func(params, returns, false) = function_type else {
        return None;
    };
    if params.len() != 1 || **returns != ParameterType::Boolean {
        return builtin_function_id(name);
    }
    match (name, &params[0]) {
        (IS_POSITIVE, ParameterType::Float) => Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT),
        (IS_POSITIVE, ParameterType::Fixed) => Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED),
        (IS_NEGATIVE, ParameterType::Float) => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT),
        (IS_NEGATIVE, ParameterType::Fixed) => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED),
        (IS_ZERO, ParameterType::Float) => Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT),
        (IS_ZERO, ParameterType::Fixed) => Some(BUILTIN_FUNCTION_IS_ZERO_FIXED),
        _ => builtin_function_id(name),
    }
}

/// The callback type a bare general builtin adopts when passed as a
/// `filter`/`transform` predicate over `element_type`, built as a
/// [`Func`](crate::types::ParameterType::Func) rather than `format!`ed
/// (plan-106-A).
///
/// plan-111-F: `resolve_call` speaks types now, so the element no longer renders
/// for the lookup, and the `String`-returning twin this used to sit beside is
/// deleted — its last caller converted in letter E.
pub(crate) fn filter_predicate_type_typed(
    name: &str,
    element_type: &crate::types::ParameterType,
) -> Option<crate::types::ParameterType> {
    use crate::types::ParameterType;
    builtin_function_id(name)?;
    let resolved = resolve_call(name, std::slice::from_ref(element_type))?;
    (resolved.return_type == ParameterType::Boolean).then(|| {
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

// ---------------------------------------------------------------------------
// Return-type resolution (argument-dependent — the bespoke resolver).
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ResolvedCall {
    pub(crate) return_type: ParameterType,
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
    // plan-111-F: `general`'s own `resolve_call` is typed too now, so the
    // render-in / parse-out pair plan-111-C left here is gone with it.
    resolve_call(name, arg_types).map(|resolved| resolved.return_type)
}

/// The static (argument-independent) nominal return of a general call — the six
/// numeric narrowing conversions carry a fixed return type; every other general call
/// resolved through `resolve_call` (`Custom`) yields `None`. Reproduces the legacy
/// `call_return_type_name` fast-oracle (`DefaultResolver::return_type_name` over the
/// `ReturnType::Fixed`/`Custom` split), consumed by `term_return_type`.
pub(crate) fn nominal_return_type(name: &str) -> Option<ParameterType> {
    match name {
        TO_INT => Some(ParameterType::Integer),
        TO_FLOAT => Some(ParameterType::Float),
        TO_FIXED => Some(ParameterType::Fixed),
        TO_BYTE => Some(ParameterType::Byte),
        TO_MONEY => Some(ParameterType::Money),
        // `Scalar` is a bare nominal, not a variant.
        TO_SCALAR => Some(ParameterType::named("Scalar")),
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

pub(crate) fn resolve_call(name: &str, arg_types: &[ParameterType]) -> Option<ResolvedCall> {
    let resolved = match name {
        ERROR => {
            if exact(arg_types, &[ParameterType::Integer, ParameterType::String]) {
                ResolvedCall {
                    return_type: ParameterType::named("Error"),
                }
            } else {
                return None;
            }
        }
        LEN => {
            if arg_types.len() != 1 {
                return None;
            }
            if arg_types[0] == ParameterType::String
                || crate::codegen::engine::types::typed_is_collection_type(&arg_types[0])
            {
                ResolvedCall {
                    return_type: ParameterType::Integer,
                }
            } else {
                return None;
            }
        }
        TYPE_NAME => {
            if arg_types.len() == 1 {
                ResolvedCall {
                    return_type: ParameterType::String,
                }
            } else {
                return None;
            }
        }
        TO_STRING => {
            // 2-arg `(Float|Fixed|Money, Byte)` precision form, or 1-arg over the nine
            // scalars plus `List OF Byte`. Both yield `String`.
            let two_arg = arg_types.len() == 2
                && matches!(
                    arg_types[0],
                    ParameterType::Float | ParameterType::Fixed | ParameterType::Money
                )
                && arg_types[1] == ParameterType::Byte;
            let one_arg = arg_types.len() == 1
                && (matches!(
                    arg_types[0],
                    ParameterType::Integer
                        | ParameterType::Float
                        | ParameterType::Fixed
                        | ParameterType::Money
                        | ParameterType::Boolean
                        | ParameterType::String
                        | ParameterType::Byte
                ) || arg_types[0].is_named("Scalar")
                    || arg_types[0].is_named("AttributedString")
                    || arg_types[0] == ParameterType::list_of(ParameterType::Byte));
            if two_arg || one_arg {
                ResolvedCall {
                    return_type: ParameterType::String,
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
                &[
                    ParameterType::String,
                    ParameterType::Byte,
                    ParameterType::Float,
                    ParameterType::Fixed,
                    ParameterType::Money,
                    ParameterType::named("Scalar"),
                ],
            ) || exact(arg_types, &[ParameterType::String, ParameterType::Integer])
            {
                ResolvedCall {
                    return_type: ParameterType::Integer,
                }
            } else {
                return None;
            }
        }
        TO_FLOAT => {
            if exact_one_of(
                arg_types,
                &[
                    ParameterType::String,
                    ParameterType::Integer,
                    ParameterType::Fixed,
                    ParameterType::Money,
                ],
            ) {
                ResolvedCall {
                    return_type: ParameterType::Float,
                }
            } else {
                return None;
            }
        }
        TO_FIXED => {
            if exact_one_of(
                arg_types,
                &[
                    ParameterType::String,
                    ParameterType::Integer,
                    ParameterType::Float,
                    ParameterType::Money,
                ],
            ) {
                ResolvedCall {
                    return_type: ParameterType::Fixed,
                }
            } else {
                return None;
            }
        }
        TO_BYTE => {
            if exact_one_of(
                arg_types,
                &[
                    ParameterType::Integer,
                    ParameterType::Money,
                    ParameterType::named("Scalar"),
                ],
            ) {
                ResolvedCall {
                    return_type: ParameterType::Byte,
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
            if exact_one_of(
                arg_types,
                &[
                    ParameterType::Integer,
                    ParameterType::String,
                    ParameterType::Byte,
                ],
            ) {
                ResolvedCall {
                    return_type: ParameterType::named("Scalar"),
                }
            } else {
                return None;
            }
        }
        TO_MONEY => {
            // Explicit crossing into Money from every scalar type (plan-29-G §4.2).
            if exact_one_of(
                arg_types,
                &[
                    ParameterType::String,
                    ParameterType::Integer,
                    ParameterType::Float,
                    ParameterType::Fixed,
                    ParameterType::Byte,
                ],
            ) {
                ResolvedCall {
                    return_type: ParameterType::Money,
                }
            } else {
                return None;
            }
        }
        IS_NUMERIC => {
            if exact(arg_types, &[ParameterType::String]) {
                ResolvedCall {
                    return_type: ParameterType::Boolean,
                }
            } else {
                return None;
            }
        }
        IS_EVEN | IS_ODD => {
            if exact(arg_types, &[ParameterType::Integer]) {
                ResolvedCall {
                    return_type: ParameterType::Boolean,
                }
            } else {
                return None;
            }
        }
        IS_POSITIVE | IS_NEGATIVE | IS_ZERO => {
            if exact_one_of(
                arg_types,
                &[
                    ParameterType::Integer,
                    ParameterType::Float,
                    ParameterType::Fixed,
                ],
            ) {
                ResolvedCall {
                    return_type: ParameterType::Boolean,
                }
            } else {
                return None;
            }
        }
        IS_EMPTY | IS_NOT_EMPTY
            if arg_types.len() == 1
                && (arg_types[0] == ParameterType::String
                    || crate::codegen::engine::types::typed_is_collection_type(&arg_types[0])) =>
        {
            ResolvedCall {
                return_type: ParameterType::Boolean,
            }
        }
        _ => return None,
    };
    Some(resolved)
}

/// Exactly the `expected` argument types, in order.
fn exact(arg_types: &[ParameterType], expected: &[ParameterType]) -> bool {
    arg_types.len() == expected.len() && arg_types.iter().zip(expected).all(|(a, e)| a == e)
}

/// Exactly one argument, of any of the `expected` types.
fn exact_one_of(arg_types: &[ParameterType], expected: &[ParameterType]) -> bool {
    arg_types.len() == 1 && expected.contains(&arg_types[0])
}

// ---------------------------------------------------------------------------
// Registration (membership / arity / declared-errors home in the clean-room
// registry). Each member carries illustrative parameter types and a
// `Body::Intrinsic` marker — resolution is `resolve_call`, codegen is the existing
// `RuntimeHelper::General` bare-name lowering.
// ---------------------------------------------------------------------------

/// A required parameter of illustrative type `ty`. `desc` is the man page's
/// Parameters-table prose.
pub(super) fn req(name: &'static str, ty: ParameterType, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty,
        default: DefaultValue::None,
    }
}

/// An optional parameter (widens arity but is never default-padded — the runtime
/// lowering selects the overload by argument count).
pub(super) fn opt(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
    desc: &'static str,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases,
        ty,
        default: DefaultValue::Optional,
    }
}

/// Build one general member: a single `Body::Intrinsic` implementation carrying its
/// illustrative signature, return type, and declared runtime errors.
/// `prose` is (intro, desc, example) — the three man-page fields.
pub(super) fn member(
    name: &'static str,
    prose: (&'static str, &'static str, &'static str),
    return_type: ParameterType,
    errors: Vec<&'static str>,
    params: Vec<Parameter>,
) -> RegistryFunction {
    let (intro, desc, example) = prose;
    RegistryFunction {
        name,
        intro,
        desc,
        example,
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

    func_error::register(&mut pkg);
    func_len::register(&mut pkg);
    func_type_name::register(&mut pkg);

    // Conversions.
    func_to_string::register(&mut pkg);
    func_to_int::register(&mut pkg);
    func_to_float::register(&mut pkg);
    func_to_fixed::register(&mut pkg);
    func_to_byte::register(&mut pkg);
    func_to_money::register(&mut pkg);
    func_to_scalar::register(&mut pkg);

    // Predicates.
    func_is_numeric::register(&mut pkg);
    func_is_even::register(&mut pkg);
    func_is_odd::register(&mut pkg);
    func_is_positive::register(&mut pkg);
    func_is_negative::register(&mut pkg);
    func_is_zero::register(&mut pkg);
    func_is_empty::register(&mut pkg);
    func_is_not_empty::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// plan-111-C: `resolve_return_type` takes and returns types now.
    fn types(items: &[&str]) -> Vec<ParameterType> {
        items.iter().map(|s| ParameterType::parse(s)).collect()
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        // The assertions read as spellings on purpose — a resolver test says
        // which type a call resolves to, and a name is how it says it. The parse
        // is here, once, at the test boundary (plan-111-C Correction C4's rule).
        resolve_call(name, &types(args)).map(|r| r.return_type.name().into_owned())
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
