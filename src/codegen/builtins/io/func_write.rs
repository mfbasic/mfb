//! `io::write` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101); the shared writer seam that
//! `io::{print,printError,writeError}` also dispatch through lives in
//! [`super::gen_write_family`].

use super::gen_write_family::lower_write_family;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `abi_function` body for `io::write` — write to stdout with no trailing newline.
/// The `String` and `AttributedString` overloads share this one helper.
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_write_family(builder, ctx, false, false, "io.write")
}

const INTRO: &str = r#"Write a `String` to standard output with no trailing newline"#;
const DESC: &str = r#"`io::write` writes `value` to standard output and adds nothing. Outside `term::` TUI
mode the text is treated as UTF-8 and emitted byte for byte, with no escaping and no
newline translation; in TUI mode it is written into the terminal grid, where line
breaks and carriage returns position the text. An empty `String` writes nothing at all. It is the
newline-free counterpart of `io::print`, which is the same call with a trailing
LF appended.

Exactly one argument, either a `String` or an `AttributedString` (see
`mfb man astrings`); there is no implicit conversion, so convert any other value
first — for example with `toString`.

In a console program every byte is written, even across short writes and
interruptions, and a write that fails or transfers nothing raises `ErrWriteFailed`.
If standard output is a pipe that the reader has already closed, the program ends
with `SIGPIPE` (exit status 141) instead of raising.

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer rather than written immediately, so it may
not be visible to an external reader until drained. The buffer is drained before an
append would exceed it, on `io::flush`, before any standard-input read, and at program exit —
which is why a prompt written with `io::write` still appears before a following
`io::readLine` even under buffering. While the program is in `term::` TUI mode,
standard output is retained rather than printed and nothing reaches the terminal
until `term::sync` presents the frame. Output goes to standard output in a console
program. In app mode (`mfb build --app`) it goes to the transcript window when there
is one, to standard output when the app runs without a window, and to the TUI
surface while TUI mode is active."#;
const EX: &str = r#"Write a prompt on the same line as the answer:

```
IMPORT io

SUB main()
  io::write("Name: ")
  LET name AS String = io::readLine()
END SUB
```

Build a line from several pieces:

```
IMPORT io

SUB main()
  io::write("x=")
  io::write(toString(3))
  io::print("")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
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
                body: Body::abi_function(lower_write),
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
                errors: vec![],
                body: Body::abi_function(lower_write),
            },
        ],
    });
}
