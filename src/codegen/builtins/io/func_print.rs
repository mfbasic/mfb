//! `io::print` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): `lower_*` emits a vreg body into the builder;
//! app mode appends the platform transcript-write sequence in place (append
//! shape); the wrapper finalizes. No hatch.

// --- codegen tier imports (migration) ---
use super::gen_write_family::lower_write_family;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `abi_function` body for `io::print` — write to stdout with a trailing newline.
/// The `String` and `AttributedString` overloads share this one helper (both hand
/// a string-object pointer to the writer).
pub(crate) fn lower_print(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_write_family(builder, ctx, false, true, "io.print")
}

const INTRO: &str = r#"Write text to standard output followed by a newline"#;
const DESC: &str = r#"`io::print` writes `value` to standard output and then appends a single line feed
(LF, byte `0x0A`). The text is treated as UTF-8 and emitted byte for byte, with no
escaping and no newline translation beyond the one trailing newline this call
adds. An empty `String` emits nothing but that newline.

Exactly one argument, either a `String` or an `AttributedString` (see
`mfb man astrings`). There is no implicit conversion, so any other value must be
converted first — for example with `toString`.

In a console program every byte is written, even across short writes and
interruptions, and a write that fails or transfers nothing raises `ErrWriteFailed`.
If standard output is a pipe that the reader has already closed, the program ends
with `SIGPIPE` (exit status 141) instead of raising.

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer instead of being written immediately; it is
drained before an append would exceed the buffer, on `io::flush`, before any standard-input read, and
at program exit. Buffering is off by default, in which case each call writes
straight through. While the program is in `term::` TUI mode, standard output is
retained rather than printed and nothing reaches the terminal until `term::sync`
presents the frame. Output goes to standard output in a console program. In app mode
(`mfb build --app`) it goes to the transcript window when there is one, to
standard output when the app runs without a window, and to the TUI surface while
TUI mode is active."#;
const EX: &str = r#"Print a line of text:

```
IMPORT io

SUB main()
  io::print("Hello")
END SUB
```

Convert a non-string value before printing:

```
IMPORT io

SUB main()
  io::print(toString(42))
  io::print("total: " & toString(42))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "print",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec!["ErrWriteFailed"],
                body: Body::abi_function(lower_print),
            },
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The attributed text to write. Only its text is written — its attributes are not rendered — as UTF-8, unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::named("AttributedString"),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec!["ErrWriteFailed"],
                body: Body::abi_function(lower_print),
            },
        ],
    });
}
