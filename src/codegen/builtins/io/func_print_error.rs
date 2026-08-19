//! `io::printError` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): this member adapter reproduces its former
//! `lower_io_helper` `match` arm and hatches the finalized OS-seam body back.

// --- codegen tier imports (migration) ---
use super::func_write::lower_write_family;
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

Standard error is **never buffered**. `io::setBuffered` controls standard output
only, so error output is always issued immediately and can never sit unseen in a
buffer; there is correspondingly no flush for standard error. It is also never
retained by `term::` TUI mode — the shadow-grid routing applies to standard
output alone — so an error message written while a TUI frame is being composed
goes straight to the terminal rather than into the frame.

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write raises `ErrOutput`. Output goes to
whatever is bound to standard error: file descriptor 2 in a console program, and
the application transcript in app mode (`mfb build --app`)."#;
const EX: &str = r#"Report a failure on the error stream:

```
IMPORT io

SUB main()
  io::printError("cannot open the input file")
END SUB
```

Colour the message only when standard error is a terminal:

```
IMPORT io

SUB main()
  IF io::isErrorTerminal() THEN
    io::printError("\u{1b}[31mError\u{1b}[0m: something went wrong")
  ELSE
    io::printError("Error: something went wrong")
  END IF
END SUB
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
            errors: vec![],
            body: Body::abi_function(lower_print_error),
        }],
    });
}
