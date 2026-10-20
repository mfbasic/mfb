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
use crate::codegen::builtins::{NoTypeKinds, TypeKinds};
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
/// user `FUNC` (plan-01-overload.md §A.5). The reserved set is exactly
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
    let resolved = resolve_call(name, std::slice::from_ref(element_type), &NoTypeKinds)?;
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
            "Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, List OF Byte, or an enum",
        ),
        TO_INT => Some("String[, Integer], Byte, Float, Fixed, Money, Scalar, or an enum"),
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
    kinds: &dyn TypeKinds,
) -> Option<crate::types::ParameterType> {
    // plan-111-F: `general`'s own `resolve_call` is typed too now, so the
    // render-in / parse-out pair plan-111-C left here is gone with it.
    resolve_call(name, arg_types, kinds).map(|resolved| resolved.return_type)
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

/// `kinds` (plan-140-A) tells an enum from any other declared type: `toInt`
/// (plan-140-B) and `toString` (plan-140-C) accept any enum.
pub(crate) fn resolve_call(
    name: &str,
    arg_types: &[ParameterType],
    kinds: &dyn TypeKinds,
) -> Option<ResolvedCall> {
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
            // scalars plus `List OF Byte` and any enum (the member's name,
            // plan-140-C). Both yield `String`.
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
                    || arg_types[0] == ParameterType::list_of(ParameterType::Byte)
                    || kinds.is_enum(&arg_types[0]));
            if two_arg || one_arg {
                ResolvedCall {
                    return_type: ParameterType::String,
                }
            } else {
                return None;
            }
        }
        TO_INT => {
            // 1-arg: parse base-10 (String), numeric narrowing (Byte/Float/Fixed),
            // or an enum's 0-based declaration index (plan-140-B). 2-arg: `toInt(text AS String, base AS Integer)` parses `text` in
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
                || (arg_types.len() == 1 && kinds.is_enum(&arg_types[0]))
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
have no `general::` spelling.

Each conversion accepts several source types, and its page lists every form. \
`toString` takes `Integer`, `Float`, `Fixed`, `Boolean`, `String`, `Byte`, `Scalar`, \
or `List OF Byte` (`Float` and `Fixed` also take an optional `Byte` second argument). \
`toInt` takes `String` (with an optional `Integer` base), `Byte`, `Float`, `Fixed`, \
`Money`, or `Scalar`. `toFloat` takes `String`, `Integer`, `Fixed`, or `Money`; \
`toFixed` takes `String`, `Integer`, `Float`, or `Money`. `toByte` takes `Integer`, \
`Money`, or `Scalar`. `toMoney` takes `String`, `Integer`, `Float`, `Fixed`, or \
`Byte`. `toScalar` takes `Integer`, `String`, or `Byte`.";

/// Register the `general` package on the clean-room registry. See the module docs for
/// why it is a real-named-but-unqualified-global package and why `error` is irregular.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("general", INTRO, DESC);
    // `general` is the globals holder for the language: its members are called bare
    // (`len(s)`) and a program never needs `general::`. Its man pages still group them
    // under `general::` — that is intended (bug-607, closed NOT A BUG). See
    // `RegistryPackage::unqualified_global`.
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
mod tests;
