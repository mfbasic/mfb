use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_STRING_BYTE_LEN, OPCODE_STRING_CASE_FOLD,
    OPCODE_STRING_CONTAINS, OPCODE_STRING_ENDS_WITH, OPCODE_STRING_GRAPHEMES, OPCODE_STRING_JOIN,
    OPCODE_STRING_LOWER, OPCODE_STRING_NORMALIZE_NFC, OPCODE_STRING_SPLIT,
    OPCODE_STRING_STARTS_WITH, OPCODE_STRING_TRIM, OPCODE_STRING_TRIM_END,
    OPCODE_STRING_TRIM_START, OPCODE_STRING_UPPER, TYPE_BOOLEAN, TYPE_INTEGER, TYPE_STRING,
};
use crate::ir::IrValue;
use std::borrow::Cow;
use std::collections::HashMap;

const PACKAGE: &str = "strings";
const TRIM: &str = "strings.trim";
const TRIM_START: &str = "strings.trimStart";
const TRIM_END: &str = "strings.trimEnd";
const UPPER: &str = "strings.upper";
const LOWER: &str = "strings.lower";
const CASE_FOLD: &str = "strings.caseFold";
const NORMALIZE_NFC: &str = "strings.normalizeNfc";
const GRAPHEMES: &str = "strings.graphemes";
const STARTS_WITH: &str = "strings.startsWith";
const ENDS_WITH: &str = "strings.endsWith";
const CONTAINS: &str = "strings.contains";
const SPLIT: &str = "strings.split";
const JOIN: &str = "strings.join";
const BYTE_LEN: &str = "strings.byteLen";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_strings_call(name: &str) -> bool {
    matches!(
        name,
        TRIM | TRIM_START
            | TRIM_END
            | UPPER
            | LOWER
            | CASE_FOLD
            | NORMALIZE_NFC
            | GRAPHEMES
            | STARTS_WITH
            | ENDS_WITH
            | CONTAINS
            | SPLIT
            | JOIN
            | BYTE_LEN
    )
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | JOIN => {
            Some("String")
        }
        GRAPHEMES | SPLIT => Some("List OF String"),
        STARTS_WITH | ENDS_WITH | CONTAINS => Some("Boolean"),
        BYTE_LEN => Some("Integer"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC
            if exact(arg_types, &["String"]) =>
        {
            Cow::Borrowed("String")
        }
        GRAPHEMES if exact(arg_types, &["String"]) => Cow::Borrowed("List OF String"),
        STARTS_WITH | ENDS_WITH | CONTAINS if exact(arg_types, &["String", "String"]) => {
            Cow::Borrowed("Boolean")
        }
        SPLIT if exact(arg_types, &["String", "String"]) => Cow::Borrowed("List OF String"),
        JOIN if exact(arg_types, &["List OF String", "String"]) => Cow::Borrowed("String"),
        BYTE_LEN if exact(arg_types, &["String"]) => Cow::Borrowed("Integer"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | GRAPHEMES
        | BYTE_LEN => Some("String"),
        STARTS_WITH | ENDS_WITH | CONTAINS | SPLIT => Some("String, String"),
        JOIN => Some("List OF String, String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | GRAPHEMES
        | BYTE_LEN => Some((1, 1)),
        STARTS_WITH | ENDS_WITH | CONTAINS | SPLIT | JOIN => Some((2, 2)),
        _ => None,
    }
}

pub(crate) fn lower_bytecode_call(
    lowerer: &mut dyn BuiltinCallLowerer,
    name: &str,
    args: &[IrValue],
    locals: &HashMap<String, ValueSlot>,
) -> Result<ValueSlot, String> {
    let lowered = args
        .iter()
        .map(|arg| lowerer.lower_value(arg, locals))
        .collect::<Result<Vec<_>, _>>()?;
    let arg_types = lowered
        .iter()
        .map(|slot| slot.type_name.clone())
        .collect::<Vec<_>>();
    let resolved = resolve_call(name, &arg_types).ok_or_else(|| {
        format!(
            "built-in `{name}` does not accept ({})",
            arg_types.join(", ")
        )
    })?;

    let dst_type_id = primitive_type_id(&resolved.return_type)
        .unwrap_or_else(|| lowerer.type_id(&resolved.return_type));
    let dst = lowerer.add_register(dst_type_id, 0);
    let mut operands = vec![dst];
    operands.extend(lowered.iter().map(|slot| slot.register));
    lowerer.push(opcode_for(name)?, operands);
    Ok(ValueSlot {
        register: dst,
        type_name: resolved.return_type.into_owned(),
    })
}

fn opcode_for(name: &str) -> Result<u16, String> {
    match name {
        TRIM => Ok(OPCODE_STRING_TRIM),
        TRIM_START => Ok(OPCODE_STRING_TRIM_START),
        TRIM_END => Ok(OPCODE_STRING_TRIM_END),
        UPPER => Ok(OPCODE_STRING_UPPER),
        LOWER => Ok(OPCODE_STRING_LOWER),
        CASE_FOLD => Ok(OPCODE_STRING_CASE_FOLD),
        NORMALIZE_NFC => Ok(OPCODE_STRING_NORMALIZE_NFC),
        GRAPHEMES => Ok(OPCODE_STRING_GRAPHEMES),
        STARTS_WITH => Ok(OPCODE_STRING_STARTS_WITH),
        ENDS_WITH => Ok(OPCODE_STRING_ENDS_WITH),
        CONTAINS => Ok(OPCODE_STRING_CONTAINS),
        SPLIT => Ok(OPCODE_STRING_SPLIT),
        JOIN => Ok(OPCODE_STRING_JOIN),
        BYTE_LEN => Ok(OPCODE_STRING_BYTE_LEN),
        _ => Err(format!("unsupported {PACKAGE} built-in `{name}`")),
    }
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}

fn primitive_type_id(type_name: &str) -> Option<u32> {
    match type_name {
        "Boolean" => Some(TYPE_BOOLEAN),
        "Integer" => Some(TYPE_INTEGER),
        "String" => Some(TYPE_STRING),
        _ => None,
    }
}
