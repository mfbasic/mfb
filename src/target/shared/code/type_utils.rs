use super::*;

/// Declared field types of every composite a `NirValue::MemberAccess` can name,
/// keyed `(owning type name, field name)`. Built by
/// `module_analysis::module_field_types` and threaded into
/// `static_nir_value_type` so a module-level walk can type `c.radius` the same
/// way the builder does. Without it a `MemberAccess` operand types as `None` and
/// every predicate built on this seam silently under-approximates (bug-363).
pub(super) type FieldTypes = HashMap<(String, String), String>;

pub(super) fn static_nir_value_type(
    value: &NirValue,
    locals: &HashMap<String, String>,
    fields: &FieldTypes,
) -> Option<String> {
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
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => locals.get(name).cloned(),
        NirValue::Binary {
            op, left, right, ..
        } => static_nir_value_type(left, locals, fields)
            .zip(static_nir_value_type(right, locals, fields))
            .map(|(left_type, right_type)| {
                numeric_binary_result_type(op, &left_type, &right_type).to_string()
            }),
        NirValue::Unary { operand, .. } => static_nir_value_type(operand, locals, fields),
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            let arg_types = args
                .iter()
                .map(|arg| static_nir_value_type(arg, locals, fields))
                .collect::<Option<Vec<_>>>()?;
            builtins::general::resolve_call(target, &arg_types)
                .map(|call| call.return_type.into_owned())
                .or_else(|| {
                    builtins::collections::resolve_call(target, &arg_types)
                        .map(|call| call.return_type.into_owned())
                })
                .or_else(|| {
                    builtins::strings::resolve_call(target, &arg_types)
                        .map(|call| call.return_type.into_owned())
                })
                .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
        }
        NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
        NirValue::ResultValue { value } => static_nir_value_type(value, locals, fields)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string)),
        NirValue::ResultError { .. } => Some("Error".to_string()),
        NirValue::MemberAccess { target, member } => {
            let target_type = static_nir_value_type(target, locals, fields)?;
            if member == "result" {
                if let Some(output_type) = builtins::thread::parent_thread_output(&target_type) {
                    return Some(format!("Result OF {output_type}"));
                }
            }
            // Record and union-variant fields, then the two `MapEntry` members —
            // the same three sources `CodeBuilder::static_type_name` consults
            // (it grew its record/union arm in bug-366), so this walk types a
            // member read exactly as the lowering that follows it will (bug-363).
            if let Some(field_type) = fields.get(&(target_type.clone(), member.clone())) {
                return Some(field_type.clone());
            }
            let (key_type, value_type) = parse_map_entry_type(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type),
                "value" => Some(value_type),
                _ => None,
            }
        }
        NirValue::RuntimeCall { .. } | NirValue::UnionWrap { .. } | NirValue::Closure { .. } => {
            None
        }
    }
}

pub(super) fn collection_type_code(type_: &str) -> Option<usize> {
    match type_ {
        "Nothing" => None,
        "Boolean" => Some(COLLECTION_TYPE_BOOLEAN),
        "Byte" => Some(COLLECTION_TYPE_BYTE),
        "Integer" => Some(COLLECTION_TYPE_INTEGER),
        "Float" => Some(COLLECTION_TYPE_FLOAT),
        "Fixed" => Some(COLLECTION_TYPE_FIXED),
        "Money" => Some(COLLECTION_TYPE_MONEY),
        "Scalar" => Some(COLLECTION_TYPE_SCALAR),
        "String" => Some(COLLECTION_TYPE_STRING),
        _ if type_.starts_with("List OF ") => Some(COLLECTION_TYPE_LIST),
        _ if type_.starts_with("Map OF ") => Some(COLLECTION_TYPE_MAP),
        _ => Some(COLLECTION_TYPE_OBJECT),
    }
}

/// Alignment, in bytes, of a packed collection payload identified by its compact
/// runtime type code. Mirrors `CodeBuilder::collection_payload_alignment` for
/// paths that carry the numeric type code rather than the type name: 8-byte
/// scalars, native collection/object pointers, and inline record/union slot
/// payloads require 8-byte alignment; 1-byte scalars and `String` bytes do not.
pub(super) fn collection_payload_alignment_for_code(code: usize) -> usize {
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

pub(super) fn local_constant_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
    fields: &FieldTypes,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types, fields).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
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
                    type_: "String".to_string(),
                    value,
                },
            )
        }
        NirValue::Binary { op, .. } if op == "&" => {
            static_string_value_with_constants(value, constants, types, fields).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
            })
        }
        _ => None,
    }
}

pub(super) fn strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
    fields: &FieldTypes,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| static_string_value_with_constants(arg, constants, types, fields))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode_backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode_backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode_backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode_backend::normalize_nfc(&value))
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
pub(super) fn binary_may_consume_float_into_exact(
    op: &str,
    left: &NirValue,
    right: &NirValue,
    types: &HashMap<String, String>,
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
    let result = numeric_binary_result_type(op, &left_type, &right_type);
    (result == numeric::TYPE_FIXED || result == numeric::TYPE_MONEY)
        && (left_type == numeric::TYPE_FLOAT || right_type == numeric::TYPE_FLOAT)
}

pub(super) fn static_primitive_text_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            // A Float/Fixed scientific-notation literal folds to its expanded
            // plain decimal (`2.5e2` -> `250`), so `toString` on a constant reads
            // the same as the equivalent plain literal (plan-28-B).
            "Float" | "Fixed" if value.contains('e') || value.contains('E') => {
                numeric::expanded_literal_text(value)
            }
            "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
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

pub(super) fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

pub(super) fn join_texts(values: &[ValueResult]) -> String {
    values
        .iter()
        .map(|value| value.text.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn is_collection_type(type_: &str) -> bool {
    type_.starts_with("List OF ") || type_.starts_with("Map OF ")
}

pub(super) fn list_element_type(type_: &str) -> Option<String> {
    let element = type_.strip_prefix("List OF ")?;
    // A `List OF RES File` element stores and is read as the bare resource pointer
    // (`File`); the `RES` ownership-axis marker is not part of the value (§15.6).
    Some(strip_res_marker(element).to_string())
}

pub(super) fn map_type_parts(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), strip_res_marker(value).to_string()))
}

/// Strip a `RES ` collection-element ownership-axis marker (`RES File` -> `File`).
pub(super) fn strip_res_marker(type_: &str) -> &str {
    type_.strip_prefix("RES ").unwrap_or(type_)
}

/// True when `type_` is a first-class function value type — a `FUNC(...) AS T`
/// or `ISOLATED FUNC(...) AS T`. A function value is a single 8-byte pointer to
/// an arena-lifetime closure object (`{code, env}`); it has **reference**
/// semantics, so it is stored, copied, and read as a bare pointer word with no
/// deep copy and no per-value free (bug-73). This mirrors the front-end
/// `is_function_type` in `target/shared/validate.rs`.
pub(super) fn is_function_type(type_: &str) -> bool {
    type_.starts_with("FUNC(") || type_.starts_with("ISOLATED FUNC(")
}

/// Byte index of the top-level `") AS "` that separates a function type's
/// parameter list from its return type — the `)` that closes the outermost
/// `FUNC(`, not one nested inside a higher-order parameter or return type. A
/// naive `split_once`/`rsplit_once(") AS ")` mis-parses e.g.
/// `FUNC(FUNC() AS Integer) AS String` (bug-175 F). `depth` is the paren nesting
/// already opened before `s` begins — 0 for a full `FUNC(...) AS T`, 1 when the
/// leading `FUNC(` has already been stripped.
fn top_level_return_arrow(s: &str, mut depth: i32) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 && s[i..].starts_with(") AS ") {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a function type's parameter list on the top-level `", "` separators
/// only, so a higher-order parameter type carrying its own `", "` (e.g.
/// `FUNC(Integer, String) AS Bool`) is kept intact (bug-175 F). Byte-identical to
/// `split(", ")` for parameter lists with no nested parens.
fn split_top_level_params(params: &str) -> Vec<String> {
    let bytes = params.as_bytes();
    let mut depth = 0i32;
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 && bytes.get(i + 1) == Some(&b' ') => {
                out.push(params[start..i].to_string());
                i += 2; // skip the ", " separator
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(params[start..].to_string());
    out
}

pub(super) fn callable_return_type(type_: &str) -> Option<String> {
    let idx = top_level_return_arrow(type_, 0)?;
    Some(type_[idx + ") AS ".len()..].to_string())
}

pub(super) fn function_type_parts(type_: &str) -> Option<(Vec<String>, String)> {
    let rest = type_.strip_prefix("FUNC(")?;
    // `rest` begins inside the parameter list, so one paren is already open.
    let idx = top_level_return_arrow(rest, 1)?;
    let params = &rest[..idx];
    let returns = &rest[idx + ") AS ".len()..];
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        split_top_level_params(params)
    };
    Some((params, returns.to_string()))
}

pub(super) fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

pub(super) fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

pub(super) fn native_immediate_value(type_: &str, value: &str) -> Result<String, String> {
    match type_ {
        "Nothing" => Ok("0".to_string()),
        "Float" => Ok(value
            .parse::<f64>()
            .map_err(|_| format!("invalid Float constant `{value}`"))?
            .to_bits()
            .to_string()),
        // Emit the 32.32 raw as its u64 bit pattern: the immediate encoder parses
        // `u64` (it loads a bit pattern, then a runtime negate handles the sign),
        // so a negative raw must not be printed with a `-`. For every non-negative
        // raw this is identical to the signed decimal; it only matters for the
        // minimum `Fixed` (raw == i64::MIN), which bug-07's fold produces directly.
        "Fixed" => Ok((numeric::fixed_raw_from_decimal(value)? as u64).to_string()),
        // Money materializes its base-10 scaled raw i64 as a u64 bit pattern, the
        // same negative-safe treatment as Fixed (the min Money raw is i64::MIN,
        // which the plan-29-B fold produces directly). (plan-29-C §4.2)
        "Money" => Ok((numeric::money_raw_from_decimal(value)? as u64).to_string()),
        // bug-286: a *negative* `Integer` const needs the same u64 bit-pattern
        // treatment as `Fixed`/`Money`, because the immediate encoders on both
        // backends parse `u64` and reject a leading `-`. Before bug-286's fold
        // in `ir::lower` no negative `Integer` const could reach here (every
        // negation kept its `Unary` shape), so this arm is reachable only for
        // the folded `i64::MIN` literal today. It is written for any negative
        // i64 so a future fold cannot reintroduce the same encoder failure.
        // A value that does not parse as i64 is passed through untouched, which
        // keeps every existing const byte-identical.
        "Integer" => Ok(match value.parse::<i64>() {
            Ok(number) if number < 0 => (number as u64).to_string(),
            _ => value.to_string(),
        }),
        _ => Ok(value.to_string()),
    }
}
