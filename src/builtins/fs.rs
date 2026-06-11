use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_IO_CLOSE, OPCODE_IO_OPEN, TYPE_FILE_HANDLE, TYPE_NOTHING,
};
use crate::ir::IrValue;
use std::borrow::Cow;
use std::collections::HashMap;

const PACKAGE: &str = "fs";

pub(crate) const FILE_TYPE: &str = "File";

const OPEN: &str = "fs.open";
const CLOSE: &str = "fs.close";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_fs_call(name: &str) -> bool {
    matches!(name, OPEN | CLOSE)
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == FILE_TYPE
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        OPEN => Some(FILE_TYPE),
        CLOSE => Some("Nothing"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        OPEN if exact(arg_types, &["String", "String"]) => Cow::Borrowed(FILE_TYPE),
        CLOSE if exact(arg_types, &[FILE_TYPE]) => Cow::Borrowed("Nothing"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        OPEN => Some("String, String"),
        CLOSE => Some(FILE_TYPE),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        OPEN => Some((2, 2)),
        CLOSE => Some((1, 1)),
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
    let operands = match name {
        OPEN => vec![dst, lowered[0].register, lowered[1].register],
        CLOSE => vec![dst, lowered[0].register],
        _ => return Err(format!("unsupported {PACKAGE} built-in `{name}`")),
    };
    lowerer.push(opcode_for(name)?, operands);
    Ok(ValueSlot {
        register: dst,
        type_name: resolved.return_type.into_owned(),
    })
}

fn opcode_for(name: &str) -> Result<u16, String> {
    match name {
        OPEN => Ok(OPCODE_IO_OPEN),
        CLOSE => Ok(OPCODE_IO_CLOSE),
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
        "Nothing" => Some(TYPE_NOTHING),
        FILE_TYPE => Some(TYPE_FILE_HANDLE),
        _ => None,
    }
}
