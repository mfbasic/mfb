//! Native compile-time constant folding over [`NirValue`].
//!
//! These predicates decide whether a NIR value is a compile-time constant the
//! native backend can fold — a string concatenation of literals, a `toString`
//! of a primitive constant, a `strings.upper`/`lower`/`caseFold`/`normalizeNfc`
//! of a static string, and so on. They are read by capability validation
//! (`validate/capabilities.rs`), symbol/import collection (`plan/symbols.rs`),
//! and function planning (`plan/function_builder.rs`) to recognise calls that
//! never reach the runtime because they fold to a literal.
//!
//! There is exactly one copy: `nir/` owns [`NirValue`] and is already a shared
//! dependency of both `validate/` and `plan/`, so the folding predicates live
//! here rather than being duplicated across the two module trees (bug-328).

use super::NirValue;
use std::collections::HashMap;

pub(crate) fn native_constant_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Binary { op, .. } if op == "&" => native_static_string_value(value, constants)
            .map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            }),
        _ => None,
    }
}

pub(crate) fn native_static_string_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_static_string_value(constant, constants)),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. } => {
            native_strings_package_static_string_value(target, args, constants)
        }
        NirValue::Binary {
            op, left, right, ..
        } if op == "&" => {
            let left = native_static_string_value(left, constants)?;
            let right = native_static_string_value(right, constants)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

pub(crate) fn native_strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| native_static_string_value(arg, constants))?;
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

pub(crate) fn native_static_graphemes_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
) -> Option<Vec<String>> {
    if target != "strings.graphemes" || args.len() != 1 {
        return None;
    }
    let value = native_static_string_value(&args[0], constants)?;
    Some(crate::unicode_backend::graphemes(&value))
}

pub(crate) fn native_primitive_text(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            // Float/Fixed constants fold to the runtime formatter's
            // default-precision rendering (2 places; bug-358, plan-28-B).
            "Float" | "Fixed" => crate::numeric::default_to_string_text(type_, value),
            "Integer" | "Byte" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_primitive_text(constant, constants)),
        NirValue::Global { .. } => None,
        _ => None,
    }
}
