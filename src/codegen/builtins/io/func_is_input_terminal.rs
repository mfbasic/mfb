//! `io::isInputTerminal` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101); the shared terminal-predicate seam lives in
//! [`super::gen_is_terminal`] (`isOutputTerminal`/`isErrorTerminal` share it).

use super::gen_is_terminal::lower_is_terminal;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::isInputTerminal` — `isatty(0)` (fd 0).
pub(crate) fn lower_is_input_terminal(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_is_terminal(builder, ctx, 0, "io.isInputTerminal")
}

const INTRO: &str = r#"Report whether standard input is an interactive terminal"#;
const DESC: &str = r#"`io::isInputTerminal` returns `TRUE` when standard input is connected to a
terminal and `FALSE` when it is redirected from a file, a pipe, or any other
non-terminal source. It takes no arguments.

When the question cannot be answered the call reports `FALSE` rather than
failing, so it never raises.

The probe inspects state only. It does not modify the stream, read any input,
or block waiting for data, so it is safe to call before deciding whether to
prompt interactively, enable line editing, or read a piped stream straight
through. In app mode the program has no real standard streams — input is served by
the application window, which is treated as an interactive console — so this call
returns `TRUE`."#;
const EX: &str = r#"Prompt only when a human is attached, otherwise read the piped stream:

```
IMPORT io

SUB main()
  IF io::isInputTerminal() THEN
    io::print(io::input("Name: "))
  ELSE
    io::print(io::readLine())
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isInputTerminal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_input_terminal),
        }],
    });
}
