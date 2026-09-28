//! `io::printError` — descriptor entry + authored docs.
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

/// `abi_function` body for `io::printError` — write to stderr with a trailing newline.
pub(crate) fn lower_print_error(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_write_family(builder, ctx, true, true, "io.printError")
}

const INTRO: &str = r#"Write a `String` to standard error followed by a newline"#;
const DESC: &str = r#"`io::printError` writes `value` to standard error and then appends a single line
feed (LF, byte `0x0A`). The text is treated as UTF-8 and emitted byte for byte,
with no escaping and no newline translation beyond the one trailing newline this
call adds. An empty `String` emits nothing but that newline.

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.

Standard error is **never buffered** by `io::setBuffered`, which controls standard
output only, so there is no flush for standard error. In a console program, error
output is written immediately and is not captured by `term::` TUI mode, so an error
message written while a TUI frame is being composed goes straight to the terminal.
In app mode, error output is handed to the application window, and while TUI mode
is active it is drawn on the TUI surface like standard output.

In a console program every byte is written, even across short writes and
interruptions, and a write that fails or transfers nothing raises `ErrWriteFailed`.
Output goes to standard error in a console program, and to the application
transcript in app mode (`mfb build --app`), where it is prefixed with `[stderr] `."#;
const EX: &str = r#"Report a failure on the error stream:

```
IMPORT io

FUNC main() AS Nothing
  io::printError("cannot open the input file")
END FUNC
```

Colour the message only when standard error is a terminal:

```
IMPORT io

FUNC main() AS Nothing
  IF io::isErrorTerminal() THEN
    io::printError("\u{1b}[31mError\u{1b}[0m: something went wrong")
  ELSE
    io::printError("Error: something went wrong")
  END IF
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "printError",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc:
                    "The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWriteFailed"],
            body: Body::abi_function(lower_print_error),
        }],
    });
}
