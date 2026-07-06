use std::borrow::Cow;

const PRINT: &str = "io.print";
const WRITE: &str = "io.write";
const PRINT_ERROR: &str = "io.printError";
const WRITE_ERROR: &str = "io.writeError";
const FLUSH: &str = "io.flush";
const IS_BUFFERED: &str = "io.isBuffered";
const SET_BUFFERED: &str = "io.setBuffered";
const INPUT: &str = "io.input";
const READ_LINE: &str = "io.readLine";
const READ_CHAR: &str = "io.readChar";
const READ_BYTE: &str = "io.readByte";
const POLL_INPUT: &str = "io.pollInput";
const IS_INPUT_TERMINAL: &str = "io.isInputTerminal";
const IS_OUTPUT_TERMINAL: &str = "io.isOutputTerminal";
const IS_ERROR_TERMINAL: &str = "io.isErrorTerminal";

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
            | IS_BUFFERED
            | SET_BUFFERED
            | INPUT
            | READ_LINE
            | READ_CHAR
            | READ_BYTE
            | POLL_INPUT
            | IS_INPUT_TERMINAL
            | IS_OUTPUT_TERMINAL
            | IS_ERROR_TERMINAL
    )
}

pub(crate) fn is_builtin_type(_name: &str) -> bool {
    false
}

pub(crate) fn builtin_type_fields(_name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    None
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => Some(&[&["value"]]),
        FLUSH | IS_BUFFERED | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => Some(&[]),
        SET_BUFFERED => Some(&[&["enabled"]]),
        INPUT => Some(&[&["prompt"]]),
        POLL_INPUT => Some(&[&["timeoutMs"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR | FLUSH | SET_BUFFERED => Some("Nothing"),
        INPUT | READ_LINE | READ_CHAR => Some("String"),
        READ_BYTE => Some("Byte"),
        POLL_INPUT => Some("Boolean"),
        IS_BUFFERED | IS_INPUT_TERMINAL | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => Some("Boolean"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR if exact(arg_types, &["String"]) => {
            Cow::Borrowed("Nothing")
        }
        FLUSH | IS_BUFFERED | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL
            if arg_types.is_empty() =>
        {
            Cow::Borrowed(call_return_type_name(name)?)
        }
        SET_BUFFERED if exact(arg_types, &["Boolean"]) => Cow::Borrowed("Nothing"),
        INPUT if arg_types.is_empty() || exact(arg_types, &["String"]) => Cow::Borrowed("String"),
        POLL_INPUT if arg_types.is_empty() || exact(arg_types, &["Integer"]) => {
            Cow::Borrowed("Boolean")
        }
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => Some("String"),
        FLUSH | IS_BUFFERED | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => Some("no arguments"),
        SET_BUFFERED => Some("Boolean"),
        INPUT => Some("String"),
        POLL_INPUT => Some("Integer"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        PRINT | WRITE | PRINT_ERROR | WRITE_ERROR => Some((1, 1)),
        FLUSH | IS_BUFFERED | READ_LINE | READ_CHAR | READ_BYTE | IS_INPUT_TERMINAL
        | IS_OUTPUT_TERMINAL | IS_ERROR_TERMINAL => Some((0, 0)),
        SET_BUFFERED => Some((1, 1)),
        INPUT | POLL_INPUT => Some((0, 1)),
        _ => None,
    }
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
