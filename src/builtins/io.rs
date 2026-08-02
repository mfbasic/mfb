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

pub(crate) fn is_io_call(name: &str) -> bool {
    DefaultResolver::contains(&IO, name)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn descriptor_constructors_execute_at_runtime() {
        // `ov`/`io_fn` are const fns used only in const context, so their bodies
        // never run at runtime and show as uncovered. Call them at runtime to
        // exercise (and pin the shape of) both constructors.
        let overload = ov(P_VALUE, "Nothing");
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.params[0].name, "value");
        assert_eq!(overload.return_type, ReturnType::Fixed("Nothing"));

        let niladic = ov(&[], "Boolean");
        assert!(niladic.params.is_empty());
        assert_eq!(niladic.return_type, ReturnType::Fixed("Boolean"));

        let func = io_fn(PRINT, "print", OV_WRITE);
        assert_eq!(func.name, PRINT);
        assert_eq!(func.doc_slug, "print");
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Helper);
        assert_eq!(func.overloads.len(), 1);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }

    #[test]
    fn expected_arguments_renders_every_arity_class() {
        assert_eq!(expected_arguments(PRINT), Some("String"));
        assert_eq!(expected_arguments(FLUSH), Some("no arguments"));
        assert_eq!(expected_arguments(SET_BUFFERED), Some("Boolean"));
        assert_eq!(expected_arguments(INPUT), Some("String"));
        assert_eq!(expected_arguments(POLL_INPUT), Some("Integer"));
        assert_eq!(expected_arguments("io.nope"), None);
    }
}
