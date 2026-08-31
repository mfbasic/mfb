// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::numeric;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashMap;
/// Declared field types of every composite a `NirValue::MemberAccess` can name,
/// keyed `(owning type name, field name)`. Built by
/// `module_analysis::module_field_types` and threaded into
/// `static_nir_value_type` so a module-level walk can type `c.radius` the same
/// way the builder does. Without it a `MemberAccess` operand types as `None` and
/// every predicate built on this seam silently under-approximates (bug-363).
pub(crate) type FieldTypes = HashMap<(String, String), ParameterType>;

pub(crate) fn static_nir_value_type(
    value: &NirValue,
    locals: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<ParameterType> {
    match value {
        NirValue::Const { type_, .. }
        | NirValue::LocalRef { type_, .. }
        | NirValue::Global { type_, .. }
        | NirValue::FunctionRef { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::UnionExtract { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::SetLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => locals.get(name).cloned(),
        NirValue::Binary {
            op, left, right, ..
        } => static_nir_value_type(left, locals, fields)
            .zip(static_nir_value_type(right, locals, fields))
            .map(|(left_type, right_type)| promoted_binary_type(op, &left_type, &right_type)),
        NirValue::Unary { operand, .. } => static_nir_value_type(operand, locals, fields),
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            let arg_types = args
                .iter()
                .map(|arg| static_nir_value_type(arg, locals, fields))
                .collect::<Option<Vec<_>>>()?;
            // plan-72-BB: the narrow `general`/`collections`/`strings` return-type
            // resolution (the argument-computed types codegen needs) goes through
            // the registry aggregate, gated to exactly that set so a broader
            // package's computed return never widens this oracle (the aggregate is
            // byte-identical to each package's own `resolve_call`). Other builtins
            // fall through to the nominal `call_return_type_name`, as before.
            // plan-104-C: the typed entry — no render/parse for `collections`
            // (the bespoke `general`/`strings` resolvers keep their string pocket
            // inside the twin); the nominal fallback parses a static descriptor
            // name, a registry-literal boundary.
            match builtins::builtin_package_name(target) {
                Some("general" | "collections" | "strings") => {
                    builtins::resolve_call_return_type_typed(target, &arg_types, false)
                }
                _ => None,
            }
            .or_else(|| builtins::call_return_type(target))
        }
        NirValue::ResultIsOk { .. } => Some(ParameterType::Boolean),
        // `Checked` annotates its SUCCESS type and this oracle echoes it, exactly
        // as the `CallResult` arm above echoes the callee's return type rather
        // than the `Result OF` wrapper: the Result-producing family reports what
        // the value delivers on the `Ok` path. The `Result OF T` the bind
        // receives is on the binding, which `NirValue::Local` resolves.
        NirValue::Checked { type_, .. } => Some(type_.clone()),
        NirValue::ResultValue { value } => match static_nir_value_type(value, locals, fields)? {
            ParameterType::ResultOf(success) => Some(*success),
            _ => None,
        },
        NirValue::ResultError { .. } => Some(ParameterType::named("Error")),
        NirValue::MemberAccess { target, member } => {
            let target_type = static_nir_value_type(target, locals, fields)?;
            if member == "result" {
                if let ParameterType::ThreadHandle {
                    worker: false, out, ..
                } = &target_type
                {
                    return Some(ParameterType::result_of((**out).clone()));
                }
            }
            // Record and union-variant fields, then the two `MapEntry` members —
            // the same three sources `CodeBuilder::static_type_name` consults
            // (it grew its record/union arm in bug-366), so this walk types a
            // member read exactly as the lowering that follows it will (bug-363).
            // `FieldTypes` keys are nominal type NAMES, so the lookup renders
            // the (scalar-cheap) name.
            if let Some(field_type) = fields.get(&(target_type.name().into_owned(), member.clone()))
            {
                return Some(field_type.clone());
            }
            let (key_type, value_type) = typed_map_entry_type_parts(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type.clone()),
                "value" => Some(value_type.clone()),
                _ => None,
            }
        }
        NirValue::RuntimeCall { .. } | NirValue::UnionWrap { .. } | NirValue::Closure { .. } => {
            None
        }
    }
}

/// The compact runtime type code for a collection payload.
///
/// plan-106-E: a closed match on the variants. The two container arms were
/// `starts_with("List OF ")` / `("Map OF ")`; a `Set` payload has never had its
/// own code and still falls to `OBJECT`, as the string form's tail did.
pub(crate) fn collection_type_code(type_: &ParameterType) -> Option<usize> {
    match type_ {
        ParameterType::Nothing => None,
        ParameterType::Boolean => Some(COLLECTION_TYPE_BOOLEAN),
        ParameterType::Byte => Some(COLLECTION_TYPE_BYTE),
        ParameterType::Integer => Some(COLLECTION_TYPE_INTEGER),
        ParameterType::Float => Some(COLLECTION_TYPE_FLOAT),
        ParameterType::Fixed => Some(COLLECTION_TYPE_FIXED),
        ParameterType::Money => Some(COLLECTION_TYPE_MONEY),
        ParameterType::String => Some(COLLECTION_TYPE_STRING),
        ParameterType::ListOf(_) => Some(COLLECTION_TYPE_LIST),
        ParameterType::MapOf(_, _) => Some(COLLECTION_TYPE_MAP),
        // `Scalar` is a nominal, not a variant.
        type_ if type_.is_named("Scalar") => Some(COLLECTION_TYPE_SCALAR),
        _ => Some(COLLECTION_TYPE_OBJECT),
    }
}

/// Alignment, in bytes, of a packed collection payload identified by its compact
/// runtime type code. Mirrors `CodeBuilder::collection_payload_alignment` for
/// paths that carry the numeric type code rather than the type name: 8-byte
/// scalars, native collection/object pointers, and inline record/union slot
/// payloads require 8-byte alignment; 1-byte scalars and `String` bytes do not.
pub(crate) fn collection_payload_alignment_for_code(code: usize) -> usize {
    match code {
        COLLECTION_TYPE_INTEGER
        | COLLECTION_TYPE_FLOAT
        | COLLECTION_TYPE_FIXED
        | COLLECTION_TYPE_MONEY
        | COLLECTION_TYPE_LIST
        | COLLECTION_TYPE_MAP
        | COLLECTION_TYPE_OBJECT => 8,
        // Scalar is a 4-byte codepoint lane (plan-41-C), a width distinct from the
        // 1-byte (Byte/Boolean/String) and 8-byte groups.
        COLLECTION_TYPE_SCALAR => 4,
        _ => 1,
    }
}

pub(crate) fn local_constant_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: ParameterType::String,
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: ParameterType::String,
                value,
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            // `typeName` folds to the argument type's SPELLING as a string
            // constant — the rendered name IS the program-visible value here.
            static_type_name_for_fold_with_types(&args[0], types, fields).map(|type_| {
                NirValue::Const {
                    type_: ParameterType::String,
                    value: type_.name().into_owned(),
                }
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if strings_package_static_string_value(target, args, constants, types, fields)
                .is_some() =>
        {
            strings_package_static_string_value(target, args, constants, types, fields).map(
                |value| NirValue::Const {
                    type_: ParameterType::String,
                    value,
                },
            )
        }
        NirValue::Binary { op, .. } if op == "&" => {
            static_string_value_with_constants(value, constants, types, fields).map(|value| {
                NirValue::Const {
                    type_: ParameterType::String,
                    value,
                }
            })
        }
        _ => None,
    }
}

pub(crate) fn strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| static_string_value_with_constants(arg, constants, types, fields))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode::backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode::backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode::backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode::backend::normalize_nfc(&value))
        }
        _ => None,
    }
}

/// Whether this binary op consumes a `Float` operand into an exact result type
/// (`Fixed` or `Money`), which makes the operand's finiteness observable and so
/// requires the `ERR_INVALID_FORMAT` message object.
///
/// Both exact types are in scope, not just `Fixed`: the spec gives `Money * Float`
/// and `Money / Float` the same non-finite-operand failure as the `Float`->`Fixed`
/// promotions (`ErrInvalidFormat`, 77050003 — see `mfb spec language types` §4.1
/// "Money"). Checking only for a `Fixed` result meant every `Money`-with-`Float`
/// expression under-reported and its module aborted at lowering with
/// "has no data object", even with plain locals for both operands (bug-366).
pub(crate) fn binary_may_consume_float_into_exact(
    op: &str,
    left: &NirValue,
    right: &NirValue,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> bool {
    if !matches!(op, "+" | "-" | "*" | "/" | "MOD" | "^") {
        return false;
    }
    let Some(left_type) = static_type_name_with_types(left, types, fields) else {
        return false;
    };
    let Some(right_type) = static_type_name_with_types(right, types, fields) else {
        return false;
    };
    let result = promoted_binary_type(op, &left_type, &right_type);
    matches!(result, ParameterType::Fixed | ParameterType::Money)
        && (left_type == ParameterType::Float || right_type == ParameterType::Float)
}

pub(crate) fn static_primitive_text_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_ {
            // A Float/Fixed constant folds to the runtime formatter's
            // default-precision rendering (2 places), so the same value prints
            // identically whether or not the argument was foldable (bug-358).
            // Scientific notation goes through the same conversions, so `2.5e2`
            // still reads the same as the plain literal (plan-28-B).
            ParameterType::Float | ParameterType::Fixed => {
                numeric::default_to_string_text(type_, value)
            }
            ParameterType::Integer | ParameterType::Byte | ParameterType::String => {
                Some(value.clone())
            }
            ParameterType::Boolean => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_primitive_text_with_constants(constant, constants)),
        _ => None,
    }
}

pub(crate) fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

pub(crate) fn join_texts(values: &[ValueResult]) -> String {
    values
        .iter()
        .map(|value| value.text.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a type SPELLING denotes a PARENT-side thread handle.
pub(crate) fn is_parent_thread_type(type_: &ParameterType) -> bool {
    matches!(type_, ParameterType::ThreadHandle { worker: false, .. })
}

// --- Typed structural twins (plan-104-C) -----------------------------------
//
// Variant-match equivalents of the string vocabulary above, for consumers that
// hold a `ParameterType` (post-`ValueResult` flip). Each mirrors its string
// twin's semantics exactly — including the RES-marker strips — and the string
// forms survive for the remaining string callers (retired when the last
// converts).

/// Typed twin of [`is_collection_type`].
pub(crate) fn typed_is_collection_type(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::ListOf(_) | ParameterType::MapOf(..) | ParameterType::SetOf(_)
    )
}

/// Typed twin of [`set_element_type`].
pub(crate) fn typed_set_element_type(type_: &ParameterType) -> Option<&ParameterType> {
    match type_ {
        ParameterType::SetOf(element) => Some(element),
        _ => None,
    }
}

/// Typed twin of [`list_element_type`] — the `RES` ownership-axis marker is
/// stripped from the element exactly as the string form does (§15.6).
pub(crate) fn typed_list_element_type(type_: &ParameterType) -> Option<&ParameterType> {
    match type_ {
        ParameterType::ListOf(element) => Some(typed_strip_res_marker(element)),
        _ => None,
    }
}

/// Typed twin of [`map_type_parts`] (the value side RES-stripped, like the
/// string form).
pub(crate) fn typed_map_type_parts(
    type_: &ParameterType,
) -> Option<(&ParameterType, &ParameterType)> {
    match type_ {
        ParameterType::MapOf(key, value) => Some((key, typed_strip_res_marker(value))),
        _ => None,
    }
}

/// Typed twin of [`strip_res_marker`].
pub(crate) fn typed_strip_res_marker(type_: &ParameterType) -> &ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner,
        other => other,
    }
}

/// Typed twin of [`callable_return_type`].
pub(crate) fn typed_callable_return_type(type_: &ParameterType) -> Option<&ParameterType> {
    match type_ {
        ParameterType::Func(_, return_type, _) => Some(return_type),
        _ => None,
    }
}

/// Typed twin of [`parse_map_entry_type`].
pub(crate) fn typed_map_entry_type_parts(
    type_: &ParameterType,
) -> Option<(&ParameterType, &ParameterType)> {
    match type_ {
        ParameterType::MapEntryOf(key, value) => Some((key, value)),
        _ => None,
    }
}

/// Whether a collection block of `type_` carries the FNV-1a hash bucket region
/// past its data region — true for `Map` and `Set` (both probe by key/element),
/// false for every `List` representation. This is the single predicate every
/// sizing / copy / free / reserve site consults instead of an inline
/// `kind == MAP` test, so a Set is never sized one way and freed another
/// (plan-63-B §3).
pub(crate) fn collection_has_buckets(type_: &ParameterType) -> bool {
    matches!(type_, ParameterType::MapOf(_, _) | ParameterType::SetOf(_))
}

/// Split a function type's parameter list on the top-level `", "` separators
/// only, so a higher-order parameter type carrying its own `", "` (e.g.
/// `FUNC(Integer, String) AS Bool`) is kept intact (bug-175 F). Byte-identical to
/// `split(", ")` for parameter lists with no nested parens.

/// The promoted result type of a binary numeric operation, defaulting a
/// non-numeric pairing to `Integer` — codegen's total flavour of the ONE
/// promotion algebra.
///
/// plan-106-E: was `promoted_binary_type`, which rendered both
/// operand names, ran the string algorithm, and re-matched the result back to a
/// variant. Its string twin `numeric_binary_result_type` is deleted with it —
/// promotion now has exactly one implementation, `numeric::typed_binary_result_type`.
///
/// The `unwrap_or(Integer)` is codegen's own defaulting, not part of the algebra:
/// `numeric` answers `None` for a non-numeric or dimensionally-invalid pairing,
/// and every caller here needs a type.
pub(crate) fn promoted_binary_type(
    operator: &str,
    left: &ParameterType,
    right: &ParameterType,
) -> ParameterType {
    numeric::typed_binary_result_type(operator, left, right).unwrap_or(ParameterType::Integer)
}

/// Whether a NIR type slot is the EMPTY nominal — the "no declared type" marker a
/// synthesized `Global` node carries, which sends the reader to the global table.
pub(crate) fn is_unset_type(type_: &ParameterType) -> bool {
    matches!(type_, ParameterType::Named(name) if name.resolve().is_empty())
}

/// The built-in `Scalar` nominal. It has no [`ParameterType`] variant (it is a
/// nominal, like `Error`), so it is spelled once here rather than at each site.
pub(crate) fn scalar_type() -> ParameterType {
    ParameterType::named("Scalar")
}

/// The built-in `Error` nominal.
pub(crate) fn error_type() -> ParameterType {
    ParameterType::named("Error")
}

pub(crate) fn native_immediate_value(type_: &ParameterType, value: &str) -> Result<String, String> {
    match type_ {
        ParameterType::Nothing => Ok("0".to_string()),
        ParameterType::Float => Ok(value
            .parse::<f64>()
            .map_err(|_| format!("invalid Float constant `{value}`"))?
            .to_bits()
            .to_string()),
        // Emit the 32.32 raw as its u64 bit pattern: the immediate encoder parses
        // `u64` (it loads a bit pattern, then a runtime negate handles the sign),
        // so a negative raw must not be printed with a `-`. For every non-negative
        // raw this is identical to the signed decimal; it only matters for the
        // minimum `Fixed` (raw == i64::MIN), which bug-07's fold produces directly.
        ParameterType::Fixed => Ok((numeric::fixed_raw_from_decimal(value)? as u64).to_string()),
        // Money materializes its base-10 scaled raw i64 as a u64 bit pattern, the
        // same negative-safe treatment as Fixed (the min Money raw is i64::MIN,
        // which the plan-29-B fold produces directly). (plan-29-C §4.2)
        ParameterType::Money => Ok((numeric::money_raw_from_decimal(value)? as u64).to_string()),
        // bug-286: a *negative* `Integer` const needs the same u64 bit-pattern
        // treatment as `Fixed`/`Money`, because the immediate encoders on both
        // backends parse `u64` and reject a leading `-`. Before bug-286's fold
        // in `ir::lower` no negative `Integer` const could reach here (every
        // negation kept its `Unary` shape), so this arm is reachable only for
        // the folded `i64::MIN` literal today. It is written for any negative
        // i64 so a future fold cannot reintroduce the same encoder failure.
        // A value that does not parse as i64 is passed through untouched, which
        // keeps every existing const byte-identical.
        ParameterType::Integer => Ok(match value.parse::<i64>() {
            Ok(number) if number < 0 => (number as u64).to_string(),
            _ => value.to_string(),
        }),
        _ => Ok(value.to_string()),
    }
}
