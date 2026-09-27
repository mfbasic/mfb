//! `io::input` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): `lower_*` emits a vreg body into the builder
//! (app-mode members `bl` a standalone GUI helper); the wrapper finalizes. No hatch.

use super::gen_read_line_family::{lower_read_line_family, timeout_parameter};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `abi_function` body for `io::input` — read a line from stdin after writing a
/// prompt (with prompt).
pub(crate) fn lower_input(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_read_line_family(builder, args, ctx, true, "io.input")
}

const INTRO: &str =
    r#"Read one line of UTF-8 text from standard input, optionally writing a prompt first"#;
const DESC: &str = r#"`io::input` optionally writes a prompt to standard output, then reads one line
from standard input and returns it as a `String` without its terminator. It
reads exactly as `io::readLine` does — what was typed, from a terminal or an
`--app` window; the bytes up to the next LF, from a pipe or a file — with one
difference:

**`io::input` reads an ordinary echoed line**: typed characters are echoed by the
terminal and the line is submitted with Return. If single-key terminal mode is
active, it restores ordinary line entry for the duration of this read. This is
the difference from `io::readLine`, which suppresses echo for the read; reach for
`io::input` when the user should see what they type.

The prompt is written verbatim — no trailing space or newline is added — and it
is written **directly**, bypassing the standard-output buffer, so it is on screen
before the program blocks. Any bytes already sitting in that buffer are drained
first, keeping the prompt in order with earlier output. An empty prompt writes
nothing at all and therefore cannot fail; `io::input()` with no argument is
exactly `io::input("")`. A genuine failure while writing the prompt raises
`ErrWriteFailed` before any input is read.

`timeoutMs` bounds the wait for the line, with the same rules and the same
guarantee as `io::readLine`: omitted waits for as long as it takes, `0` makes
one immediate attempt, a positive value waits that many milliseconds, a
negative one raises `ErrInvalidArgument`, and a wait that runs out raises
`ErrTimeout` with no part of the line lost. The timeout comes after the prompt,
so a timed read with no prompt is `io::input("", 5000)` — or simply
`io::readLine(5000)`, which is the same read without the echo.

The text must be valid UTF-8; an ill-formed sequence raises `ErrEncoding` rather
than yielding a replacement character. End of input is an error rather than an
empty result, but only when it arrives before any character of the line: if
input ends after part of a line, that part is returned as the final unterminated
line and the following call raises `ErrEndOfFile`. A worker thread must
subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`. In app mode the prompt and the entered line are handled by
the application window."#;
const EX: &str = r#"Prompt on the same line and greet the user:

```
IMPORT io

SUB main()
  LET name AS String = io::input("Name: ")
  io::print("Hello, " & name)
END SUB
```

Read a line without a prompt:

```
IMPORT io

SUB main()
  LET line AS String = io::input()
  io::print(line)
END SUB
```

Ask with a time limit:

```
IMPORT errorCode
IMPORT io

SUB main()
  LET name AS String = io::input("Name (10 s): ", 10000) TRAP(e)
    IF e.code = errorCode::ErrTimeout THEN
      io::print("")
      io::print("Hello, stranger")
      EXIT SUB
    END IF
    FAIL e
  END TRAP
  io::print("Hello, " & name)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "input",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "prompt",
                desc: "Optional. Text written to standard output before the read, verbatim and with nothing appended. An empty `String` (or omitting the argument) writes nothing.",
                aliases: &[],
                ty: ParameterType::String,
                // Padded at the call: `io::input()` is `io::input("")`, and the
                // timeout after it is padded too.
                default: DefaultValue::Fill {
                    type_name: ParameterType::String,
                    expr: "",
                },
            },
            timeout_parameter()],
            return_type: ParameterType::String,
            errors: vec![
                "ErrTimeout",
                "ErrInvalidArgument",
                "ErrWriteFailed",
                "ErrEndOfFile",
                "ErrEncoding",
                "ErrInputFailed",
                "ErrInvalidContext",
                "ErrOutOfMemory",
            ],
            body: Body::abi_function(lower_input),
        }],
    });
}
