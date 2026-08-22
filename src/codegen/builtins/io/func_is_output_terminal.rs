//! `io::isOutputTerminal` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): `lower_*` emits a vreg body into the builder
//! (app-mode members `bl` a standalone GUI helper); the wrapper finalizes. No hatch.

use super::func_is_input_terminal::lower_is_terminal_common;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::isOutputTerminal` — `isatty(1)` (fd 1).
pub(crate) fn lower_is_output_terminal(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_is_terminal_common(builder, ctx, 1, "io.isOutputTerminal")
}

const INTRO: &str = r#"Report whether standard output is an interactive terminal"#;
const DESC: &str = r#"`io::isOutputTerminal` returns `TRUE` when standard output is connected to a
terminal and `FALSE` when it is redirected to a file, a pipe, or any other
non-terminal destination. It takes no arguments.

The answer comes from an `isatty` probe of file descriptor 1: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.

The probe inspects state only: it writes nothing and changes nothing. Use it to
decide whether emitting ANSI colour, progress bars, or cursor tricks is
appropriate, and to fall back to plain text when output is being captured. The
answer says nothing about `io::setBuffered`: buffering is an MFBASIC-level setting
the program controls, not something inferred from whether standard output is a
terminal. In app mode the program has no real standard streams — output is
rendered by the application transcript window, which is treated as an interactive
console — so this call returns `TRUE` without probing a descriptor."#;
const EX: &str = r#"Colour the output only when a terminal is attached:

```
IMPORT io

SUB main()
  IF io::isOutputTerminal() THEN
    io::print("\u{1b}[32mStatus: OK\u{1b}[0m")
  ELSE
    io::print("Status: OK")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isOutputTerminal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_output_terminal),
        }],
    });
}
