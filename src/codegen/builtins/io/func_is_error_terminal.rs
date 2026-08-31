//! `io::isErrorTerminal` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101); the shared terminal-predicate seam lives in
//! [`super::gen_is_terminal`].

use super::gen_is_terminal::lower_is_terminal;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::isErrorTerminal` — `isatty(2)` (fd 2).
pub(crate) fn lower_is_error_terminal(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_is_terminal(builder, ctx, 2, "io.isErrorTerminal")
}

const INTRO: &str = r#"Report whether standard error is an interactive terminal"#;
const DESC: &str = r#"`io::isErrorTerminal` returns `TRUE` when standard error is connected to a
terminal and `FALSE` when it is redirected to a file, a pipe, or any other
non-terminal destination. It takes no arguments.

When the question cannot be answered the call reports `FALSE` rather than
failing, so it never raises.

Standard error is probed independently of standard output, which matters in the
common case where one is redirected and the other is not: a program run as
`prog > out.txt` should still colour its diagnostics, and `prog 2> log.txt`
should not. Ask this question about the stream you are about to write to. The
probe inspects state only: it writes nothing and changes nothing. In app mode the
program has no real standard streams — error output is rendered by the application
transcript, which is treated as an interactive console — so this call returns
`TRUE` without probing a descriptor."#;
const EX: &str = r#"Colour diagnostics only when the error stream is a terminal:

```
IMPORT io

SUB main()
  IF io::isErrorTerminal() THEN
    io::printError("\u{1b}[31mError\u{1b}[0m: build failed")
  ELSE
    io::printError("Error: build failed")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isErrorTerminal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_error_terminal),
        }],
    });
}
