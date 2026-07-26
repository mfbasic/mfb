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

use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn types(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &types(args)).map(|call| call.return_type.into_owned())
    }

    #[test]
    fn is_io_call_covers_the_package_surface() {
        for name in [
            PRINT,
            WRITE,
            PRINT_ERROR,
            WRITE_ERROR,
            FLUSH,
            IS_BUFFERED,
            SET_BUFFERED,
            INPUT,
            READ_LINE,
            READ_CHAR,
            READ_BYTE,
            POLL_INPUT,
            IS_INPUT_TERMINAL,
            IS_OUTPUT_TERMINAL,
            IS_ERROR_TERMINAL,
        ] {
            assert!(is_io_call(name), "{name}");
        }
        assert!(!is_io_call("io.nope"));
        assert!(!is_io_call("print"));
    }

    #[test]
    fn resolve_call_writers_take_one_string() {
        for name in [PRINT, WRITE, PRINT_ERROR, WRITE_ERROR] {
            assert_eq!(rt(name, &["String"]), Some("Nothing".to_string()));
            // Wrong arg type / arity does not resolve.
            assert!(rt(name, &["Integer"]).is_none());
            assert!(rt(name, &[]).is_none());
        }
    }

    #[test]
    fn resolve_call_niladic_queries() {
        for (name, ret) in [
            (FLUSH, "Nothing"),
            (IS_BUFFERED, "Boolean"),
            (READ_LINE, "String"),
            (READ_CHAR, "String"),
            (READ_BYTE, "Byte"),
            (IS_INPUT_TERMINAL, "Boolean"),
            (IS_OUTPUT_TERMINAL, "Boolean"),
            (IS_ERROR_TERMINAL, "Boolean"),
        ] {
            assert_eq!(rt(name, &[]), Some(ret.to_string()), "{name}");
            assert!(rt(name, &["String"]).is_none(), "{name} takes no arguments");
        }
    }

    #[test]
    fn resolve_call_optional_and_typed_argument_forms() {
        // input: () or (String) -> String; anything else fails.
        assert_eq!(rt(INPUT, &[]), Some("String".to_string()));
        assert_eq!(rt(INPUT, &["String"]), Some("String".to_string()));
        assert!(rt(INPUT, &["Integer"]).is_none());
        // pollInput: () or (Integer) -> Boolean.
        assert_eq!(rt(POLL_INPUT, &[]), Some("Boolean".to_string()));
        assert_eq!(rt(POLL_INPUT, &["Integer"]), Some("Boolean".to_string()));
        assert!(rt(POLL_INPUT, &["String"]).is_none());
        // setBuffered: (Boolean) -> Nothing.
        assert_eq!(rt(SET_BUFFERED, &["Boolean"]), Some("Nothing".to_string()));
        assert!(rt(SET_BUFFERED, &[]).is_none());
        assert!(rt("io.nope", &[]).is_none());
    }

    #[test]
    fn metadata_tables_agree() {
        assert_eq!(call_return_type_name(READ_BYTE), Some("Byte"));
        assert_eq!(call_return_type_name("io.nope"), None);

        assert_eq!(expected_arguments(PRINT), Some("String"));
        assert_eq!(expected_arguments(FLUSH), Some("no arguments"));
        assert_eq!(expected_arguments(POLL_INPUT), Some("Integer"));
        assert_eq!(expected_arguments("io.nope"), None);

        assert_eq!(arity(PRINT), Some((1, 1)));
        assert_eq!(arity(FLUSH), Some((0, 0)));
        assert_eq!(arity(INPUT), Some((0, 1)));
        assert_eq!(arity("io.nope"), None);

        assert_eq!(call_param_names(SET_BUFFERED).map(|groups| groups[0][0]), Some("enabled"));
        assert_eq!(call_param_names(INPUT).map(|groups| groups[0][0]), Some("prompt"));
        assert_eq!(call_param_names(POLL_INPUT).map(|groups| groups[0][0]), Some("timeoutMs"));
        assert!(call_param_names(FLUSH).is_some()); // niladic: an empty group list
        assert!(call_param_names("io.nope").is_none());

        // io exposes no value types.
        assert!(!is_builtin_type("Anything"));
        assert!(builtin_type_fields("Anything").is_none());
    }
}
