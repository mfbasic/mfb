use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_THREAD_CANCEL, OPCODE_THREAD_EMIT,
    OPCODE_THREAD_IS_CANCELLED, OPCODE_THREAD_IS_RUNNING, OPCODE_THREAD_POLL, OPCODE_THREAD_READ,
    OPCODE_THREAD_RECEIVE, OPCODE_THREAD_SEND, OPCODE_THREAD_START, OPCODE_THREAD_WAIT_FOR,
    TYPE_BOOLEAN, TYPE_NOTHING,
};
use crate::ir::IrValue;
use std::borrow::Cow;
use std::collections::HashMap;

const PACKAGE: &str = "thread";

pub(crate) const THREAD_TYPE: &str = "Thread";

const START: &str = "thread.start";
const IS_RUNNING: &str = "thread.isRunning";
const WAIT_FOR: &str = "thread.waitFor";
const CANCEL: &str = "thread.cancel";
const SEND: &str = "thread.send";
const POLL: &str = "thread.poll";
const READ: &str = "thread.read";
const RECEIVE: &str = "thread.receive";
const EMIT: &str = "thread.emit";
const IS_CANCELLED: &str = "thread.isCancelled";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_thread_call(name: &str) -> bool {
    matches!(
        name,
        START | IS_RUNNING | WAIT_FOR | CANCEL | SEND | POLL | READ | RECEIVE | EMIT | IS_CANCELLED
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == THREAD_TYPE || name.starts_with("Thread OF ")
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        START if matches_start(arg_types) => {
            let output = function_output(&arg_types[0])?;
            Cow::Owned(format!("Thread OF Unknown TO {output}"))
        }
        IS_RUNNING if arg_types.len() == 1 && is_thread_type(&arg_types[0]) => {
            Cow::Borrowed("Boolean")
        }
        WAIT_FOR if arg_types.len() == 1 => thread_output(&arg_types[0]).map(Cow::Borrowed)?,
        CANCEL if arg_types.len() == 1 && is_thread_type(&arg_types[0]) => Cow::Borrowed("Nothing"),
        SEND if (arg_types.len() == 2 || arg_types.len() == 3)
            && is_thread_type(&arg_types[0])
            && thread_message(&arg_types[0])
                .is_some_and(|message| message == "Unknown" || message == arg_types[1])
            && arg_types.get(2).is_none_or(|timeout| timeout == "Integer") =>
        {
            Cow::Borrowed("Nothing")
        }
        POLL if arg_types.len() == 2
            && is_thread_type(&arg_types[0])
            && arg_types[1] == "Integer" =>
        {
            Cow::Borrowed("Boolean")
        }
        READ if arg_types.len() == 1 => thread_message(&arg_types[0]).map(Cow::Borrowed)?,
        RECEIVE if arg_types.is_empty() || exact(arg_types, &["Integer"]) => {
            Cow::Borrowed("Unknown")
        }
        EMIT if (arg_types.len() == 1 || arg_types.len() == 2)
            && arg_types.get(1).is_none_or(|timeout| timeout == "Integer") =>
        {
            Cow::Borrowed("Nothing")
        }
        IS_CANCELLED if arg_types.is_empty() => Cow::Borrowed("Boolean"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        START => Some("ISOLATED FUNC(In) AS Out, In, Integer, Integer"),
        IS_RUNNING | WAIT_FOR | CANCEL | READ => Some("Thread OF Msg TO Out"),
        SEND => Some("Thread OF Msg TO Out, Msg, Integer"),
        POLL => Some("Thread OF Msg TO Out, Integer"),
        RECEIVE => Some("Integer"),
        EMIT => Some("Msg, Integer"),
        IS_CANCELLED => Some("no arguments"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        START => Some((2, 4)),
        IS_RUNNING | WAIT_FOR | CANCEL | READ => Some((1, 1)),
        SEND => Some((2, 3)),
        POLL => Some((2, 2)),
        RECEIVE => Some((0, 1)),
        EMIT => Some((1, 2)),
        IS_CANCELLED => Some((0, 0)),
        _ => None,
    }
}

pub(crate) fn lower_bytecode_call(
    lowerer: &mut dyn BuiltinCallLowerer,
    name: &str,
    args: &[IrValue],
    locals: &HashMap<String, ValueSlot>,
) -> Result<ValueSlot, String> {
    let mut lowered = args
        .iter()
        .map(|arg| lowerer.lower_value(arg, locals))
        .collect::<Result<Vec<_>, _>>()?;

    match name {
        START if lowered.len() == 2 => {
            lowered.push(lowerer.push_integer_const(64)?);
            lowered.push(lowerer.push_integer_const(64)?);
        }
        START if lowered.len() == 3 => {
            lowered.push(lowerer.push_integer_const(64)?);
        }
        SEND | RECEIVE | EMIT if lowered.len() == arity(name).map(|(min, _)| min).unwrap_or(0) => {
            lowered.push(lowerer.push_integer_const(0)?);
        }
        _ => {}
    }

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
        START => Ok(OPCODE_THREAD_START),
        IS_RUNNING => Ok(OPCODE_THREAD_IS_RUNNING),
        WAIT_FOR => Ok(OPCODE_THREAD_WAIT_FOR),
        CANCEL => Ok(OPCODE_THREAD_CANCEL),
        SEND => Ok(OPCODE_THREAD_SEND),
        POLL => Ok(OPCODE_THREAD_POLL),
        READ => Ok(OPCODE_THREAD_READ),
        RECEIVE => Ok(OPCODE_THREAD_RECEIVE),
        EMIT => Ok(OPCODE_THREAD_EMIT),
        IS_CANCELLED => Ok(OPCODE_THREAD_IS_CANCELLED),
        _ => Err(format!("unsupported {PACKAGE} built-in `{name}`")),
    }
}

fn matches_start(arg_types: &[String]) -> bool {
    if !(2..=4).contains(&arg_types.len()) {
        return false;
    }
    function_input(&arg_types[0]).is_some_and(|input| input == arg_types[1])
        && arg_types.get(2).is_none_or(|limit| limit == "Integer")
        && arg_types.get(3).is_none_or(|limit| limit == "Integer")
}

fn function_input(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("ISOLATED FUNC(")?;
    rest.split_once(") AS ").map(|(params, _)| params)
}

fn function_output(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("ISOLATED FUNC(")?;
    rest.split_once(") AS ").map(|(_, output)| output)
}

pub(crate) fn is_thread_type(name: &str) -> bool {
    thread_parts(name).is_some()
}

pub(crate) fn thread_message(name: &str) -> Option<&str> {
    thread_parts(name).map(|(message, _)| message)
}

pub(crate) fn thread_output(name: &str) -> Option<&str> {
    thread_parts(name).map(|(_, output)| output)
}

pub(crate) fn thread_parts(name: &str) -> Option<(&str, &str)> {
    name.strip_prefix("Thread OF ")?.split_once(" TO ")
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
        _ => None,
    }
}
