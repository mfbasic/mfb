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

const INTRO: &str = r#"Write a `String` to standard output followed by a newline"#;
const DESC: &str = r#"`io::print` writes `value` to standard output and then appends a single line feed
(LF, byte `0x0A`). The text is treated as UTF-8 and emitted byte for byte, with no
escaping and no newline translation beyond the one trailing newline this call
adds. An empty `String` emits nothing but that newline.

Only `String` is accepted, and exactly one argument. There is no implicit
conversion, so a non-string value must be converted first — for example with
`toString`.

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write is a failure and raises
`ErrOutput`.

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer instead of being written immediately; it is
drained when the buffer fills, on `io::flush`, before any standard-input read, and
at program exit. Buffering is off by default, in which case each call writes
straight through. While the program is in `term::` TUI mode, standard output is
retained rather than printed and nothing reaches the terminal until `term::sync`
presents the frame. Output goes to whatever is bound to standard output: file
descriptor 1 in a console program, and the application transcript window in app
mode (`mfb build --app`)."#;
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
                errors: vec![],
                body: Body::abi_function(lower_print),
            },
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The attributed text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::Named("AttributedString"),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function(lower_print),
            },
        ],
    });
}
