use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, DefaultResolver, DefaultValue,
    Implementation, Lowering, Parameter, ParameterType, ReturnType,
};

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

// plan-72-N: `IO` is the descriptor authority for this package. Every call lowers
// to a runtime helper (no implementation-name rewrite) with a single fixed-return
// overload: the writers take one `String`, the terminal/buffer queries are
// niladic, and `input`/`pollInput` take one optional trailing argument
// (arity 0..1) modelled as `DefaultValue::Optional` — it widens arity but is NOT
// default-padded, because io has no default-padding helper. io contributes no
// builtin types (its `is_builtin_type`/`builtin_type_fields` were constant
// `false`/`None` stubs, not real types — see Corrections in plan-72-N) and has no
// source companion or custom resolver.
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn io_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const P_VALUE: &[Parameter] = &[Parameter::required("value", "String")];
const P_ENABLED: &[Parameter] = &[Parameter::required("enabled", "Boolean")];
// `prompt`/`timeoutMs` are optional trailing arguments: present or absent, no
// injected literal (Optional, not Fill).
const P_PROMPT: &[Parameter] = &[Parameter {
    name: "prompt",
    aliases: &[],
    ty: ParameterType::Named("String"),
    default: DefaultValue::Optional,
}];
const P_TIMEOUT: &[Parameter] = &[Parameter {
    name: "timeoutMs",
    aliases: &[],
    ty: ParameterType::Named("Integer"),
    default: DefaultValue::Optional,
}];

const OV_WRITE: &[BuiltinOverload] = &[ov(P_VALUE, "Nothing")];
const OV_NIL_NOTHING: &[BuiltinOverload] = &[ov(&[], "Nothing")];
const OV_NIL_BOOL: &[BuiltinOverload] = &[ov(&[], "Boolean")];
const OV_NIL_STRING: &[BuiltinOverload] = &[ov(&[], "String")];
const OV_NIL_BYTE: &[BuiltinOverload] = &[ov(&[], "Byte")];
const OV_SET_BUFFERED: &[BuiltinOverload] = &[ov(P_ENABLED, "Nothing")];
const OV_INPUT: &[BuiltinOverload] = &[ov(P_PROMPT, "String")];
const OV_POLL: &[BuiltinOverload] = &[ov(P_TIMEOUT, "Boolean")];

const IO_FUNCTIONS: &[BuiltinFunction] = &[
    io_fn(PRINT, "print", OV_WRITE),
    io_fn(WRITE, "write", OV_WRITE),
    io_fn(PRINT_ERROR, "printError", OV_WRITE),
    io_fn(WRITE_ERROR, "writeError", OV_WRITE),
    io_fn(FLUSH, "flush", OV_NIL_NOTHING),
    io_fn(IS_BUFFERED, "isBuffered", OV_NIL_BOOL),
    io_fn(SET_BUFFERED, "setBuffered", OV_SET_BUFFERED),
    io_fn(INPUT, "input", OV_INPUT),
    io_fn(READ_LINE, "readLine", OV_NIL_STRING),
    io_fn(READ_CHAR, "readChar", OV_NIL_STRING),
    io_fn(READ_BYTE, "readByte", OV_NIL_BYTE),
    io_fn(POLL_INPUT, "pollInput", OV_POLL),
    io_fn(IS_INPUT_TERMINAL, "isInputTerminal", OV_NIL_BOOL),
    io_fn(IS_OUTPUT_TERMINAL, "isOutputTerminal", OV_NIL_BOOL),
    io_fn(IS_ERROR_TERMINAL, "isErrorTerminal", OV_NIL_BOOL),
];

pub(crate) static IO: BuiltinModule = BuiltinModule {
    name: "io",
    functions: IO_FUNCTIONS,
    types: &[],
    source: None,
    resolver: None,
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_io_call(name: &str) -> bool {
    DefaultResolver::contains(&IO, name)
}

/// io contributes no value types; these query `IO.types` (empty) so they stay the
/// descriptor's authority and always report absence.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    IO.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    IO.types
        .iter()
        .find(|ty| ty.name == name)
        .map(|ty| ty.fields)
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver` (which yields `Vec`) cannot produce, and its consumers require
// the borrow, so it stays a static literal PINNED equal to `IO` by
// `parity_matches_descriptor` until plan-72-BB moves the consumers onto the owned
// descriptor API.
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
    DefaultResolver::return_type_name(&IO, name)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    DefaultResolver::resolve_call(&IO, name, arg_types).map(|return_type| ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

// The niladic io calls render their expected arguments as `"no arguments"`, a
// bespoke phrasing the descriptor's per-position type rendering (`"()"`) cannot
// reproduce, so this stays a hand-authored static (not descriptor-derived) and the
// parity harness opts out of the `expected_arguments` row for io. BB removes it.
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
    DefaultResolver::arity(&IO, name)
}

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

        assert_eq!(
            call_param_names(SET_BUFFERED).map(|groups| groups[0][0]),
            Some("enabled")
        );
        assert_eq!(
            call_param_names(INPUT).map(|groups| groups[0][0]),
            Some("prompt")
        );
        assert_eq!(
            call_param_names(POLL_INPUT).map(|groups| groups[0][0]),
            Some("timeoutMs")
        );
        assert!(call_param_names(FLUSH).is_some()); // niladic: an empty group list
        assert!(call_param_names("io.nope").is_none());

        // io exposes no value types.
        assert!(!is_builtin_type("Anything"));
        assert!(builtin_type_fields("Anything").is_none());
    }

    // plan-72-N migration gate: prove `IO` reproduces every legacy helper answer
    // for every `io.*` name (and an unknown name) — membership, arity, param
    // names, return type — pins the `call_param_names` static equal to `IO`, and
    // checks `resolve_call` across the writer/niladic/optional-argument shapes.
    // `expected_arguments` is opted out (bespoke "no arguments" phrasing) and kept
    // hand-authored. Keep until plan-72-BB deletes the legacy helpers.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let calls: Vec<&str> = IO_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_io_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &call_return_type_name,
            // io renders niladic calls as "no arguments", which the descriptor's
            // per-position rendering ("()") cannot reproduce; opt out and keep the
            // hand-authored strings.
            expected_arguments: None,
            param_name_overloads: None,
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: None,
        };
        let mut probe = calls.clone();
        probe.push("io.nope");
        parity::assert_parity(&IO, &probe, &legacy, &[]);

        // resolve_call parity across the writer/niladic/optional-argument shapes.
        assert_eq!(rt(PRINT, &["String"]), Some("Nothing".to_string()));
        assert!(rt(PRINT, &[]).is_none());
        assert_eq!(rt(FLUSH, &[]), Some("Nothing".to_string()));
        assert!(rt(FLUSH, &["String"]).is_none());
        assert_eq!(rt(INPUT, &[]), Some("String".to_string()));
        assert_eq!(rt(INPUT, &["String"]), Some("String".to_string()));
        assert!(rt(INPUT, &["Integer"]).is_none());
        assert_eq!(rt(POLL_INPUT, &["Integer"]), Some("Boolean".to_string()));
        assert!(rt(POLL_INPUT, &["String"]).is_none());
        assert_eq!(rt(SET_BUFFERED, &["Boolean"]), Some("Nothing".to_string()));

        // io contributes no builtin types.
        assert!(IO.types.is_empty());
    }
}
