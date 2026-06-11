use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_IO_FLUSH, OPCODE_IO_IS_TERMINAL, OPCODE_IO_READ_BYTE,
    OPCODE_IO_READ_CHAR, OPCODE_IO_READ_LINE, OPCODE_IO_TERMINAL_SIZE, OPCODE_IO_WRITE,
    TYPE_BOOLEAN, TYPE_BYTE, TYPE_NOTHING, TYPE_STRING,
};
use crate::ir::IrValue;
use std::borrow::Cow;
use std::collections::HashMap;

const PACKAGE: &str = "io";

pub(crate) const TERMINAL_SIZE_TYPE: &str = "TerminalSize";

const PRINT: &str = "io.print";
const WRITE: &str = "io.write";
const PRINT_ERROR: &str = "io.printError";
const WRITE_ERROR: &str = "io.writeError";
const FLUSH: &str = "io.flush";
const FLUSH_ERROR: &str = "io.flushError";
const INPUT: &str = "io.input";
const READ_LINE: &str = "io.readLine";
const READ_CHAR: &str = "io.readChar";
const READ_BYTE: &str = "io.readByte";
const IS_INPUT_TERMINAL: &str = "io.isInputTerminal";
const IS_OUTPUT_TERMINAL: &str = "io.isOutputTerminal";
const IS_ERROR_TERMINAL: &str = "io.isErrorTerminal";
const TERMINAL_SIZE: &str = "io.terminalSize";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_io_call(name: &str) -> bool {
    matches!(
        name,
        PRINT
            | WRITE
            | PRINT_ERROR
            | WRITE_ERROR
            | FLUSH
            | FLUSH_ERROR
            | INPUT
            | READ_LINE
            | READ_CHAR
            | READ_BYTE
            | IS_INPUT_TERMINAL
            | IS_OUTPUT_TERMINAL
            | IS_ERROR_TERMINAL
            | TERMINAL_SIZE
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == TERMINAL_SIZE_TYPE
}

pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match name {
        TERMINAL_SIZE_TYPE => Some(&[("columns", "Integer"), ("rows", "Integer")]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR | FLUSH | FLUSH_ERROR => Some("Nothing"),
        INPUT | READ_LINE | READ_CHAR => Some("String"),
        READ_BYTE => Some("Byte"),
        IS_INPUT_TERMINAL | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => Some("Boolean"),
        TERMINAL_SIZE => Some(TERMINAL_SIZE_TYPE),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR if exact(arg_types, &["String"]) => {
            Cow::Borrowed("Nothing")
        }
        FLUSH | FLUSH_ERROR | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL | TERMINAL_SIZE
            if arg_types.is_empty() =>
        {
            Cow::Borrowed(call_return_type_name(name)?)
        }
        INPUT if arg_types.is_empty() || exact(arg_types, &["String"]) => Cow::Borrowed("String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => Some("String"),
        FLUSH | FLUSH_ERROR | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL | TERMINAL_SIZE => Some("no arguments"),
        INPUT => Some("String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => Some((1, 1)),
        FLUSH | FLUSH_ERROR | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL | TERMINAL_SIZE => Some((0, 0)),
        INPUT => Some((0, 1)),
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
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => {
            vec![
                dst,
                lowered[0].register,
                fd_for(name),
                u32::from(appends_newline(name)),
            ]
        }
        FLUSH | FLUSH_ERROR => vec![dst, fd_for(name)],
        INPUT => {
            let prompt = lowered
                .first()
                .map(|slot| slot.register)
                .unwrap_or(u32::MAX);
            vec![dst, prompt]
        }
        READ_LINE | READ_CHAR | READ_BYTE | TERMINAL_SIZE => vec![dst],
        IS_INPUT_TERMINAL | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => vec![dst, fd_for(name)],
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
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => Ok(OPCODE_IO_WRITE),
        FLUSH | FLUSH_ERROR => Ok(OPCODE_IO_FLUSH),
        INPUT | READ_LINE => Ok(OPCODE_IO_READ_LINE),
        READ_CHAR => Ok(OPCODE_IO_READ_CHAR),
        READ_BYTE => Ok(OPCODE_IO_READ_BYTE),
        IS_INPUT_TERMINAL | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => Ok(OPCODE_IO_IS_TERMINAL),
        TERMINAL_SIZE => Ok(OPCODE_IO_TERMINAL_SIZE),
        _ => Err(format!("unsupported {PACKAGE} built-in `{name}`")),
    }
}

fn fd_for(name: &str) -> u32 {
    match name {
        PRINT_ERROR | WRITE_ERROR | FLUSH_ERROR | IS_ERROR_TERMINAL => 2,
        IS_INPUT_TERMINAL => 0,
        _ => 1,
    }
}

fn appends_newline(name: &str) -> bool {
    matches!(name, PRINT | PRINT_ERROR)
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
        "Boolean" => Some(TYPE_BOOLEAN),
        "Byte" => Some(TYPE_BYTE),
        "String" => Some(TYPE_STRING),
        _ => None,
    }
}
