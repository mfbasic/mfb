//! `io::isBuffered` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_is_buffered`] reproduces this
//! member's former `lower_io_helper` `match` arm — branch app-vs-console on the
//! threaded `AbiCtx`, call the OS-seam emitter, and hand the finalized body back
//! through the pre-finalized hatch.

use crate::codegen::builtins::io::native::{
    adapter_app_mode, hatch_finalized, lower_io_is_buffered_helper,
};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::isBuffered` (no args). Reads the thread stdout
/// buffering flag (or, in app mode, always `FALSE`) via
/// `lower_io_is_buffered_helper`, hatched back pre-finalized.
pub(crate) fn lower_is_buffered(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let body = lower_io_is_buffered_helper(&symbol, adapter_app_mode(ctx))?;
    hatch_finalized(builder, body, "Boolean", "io.isBuffered")
}

const INTRO: &str = r#"Report whether standard-output buffering is enabled for this thread"#;
const DESC: &str = r#"`io::isBuffered` returns `TRUE` when opt-in standard-output buffering is on for
the calling thread and `FALSE` otherwise. It takes no arguments.

The result is the thread's buffering flag read directly: `TRUE` after
`io::setBuffered(TRUE)`, `FALSE` after `io::setBuffered(FALSE)`. Buffering is off
by default, so a program that never calls `io::setBuffered` always observes
`FALSE`.

The flag is per thread — each thread has its own standard-output buffer and its
own enabled state — so this call never reports another thread's mode. Standard
error is never buffered and has no corresponding query.

The call reads state only: it writes nothing, drains nothing, and cannot fail. In
app mode the standard-output buffer is inert, so `io::isBuffered` always reports
`FALSE` there regardless of any `io::setBuffered` call."#;
const EX: &str = r#"Enable buffering only when it is not already on:

```
IMPORT io

SUB main()
  IF NOT io::isBuffered() THEN
    io::setBuffered(TRUE)
  END IF
END SUB
```

Capture the mode so it can be restored later:

```
IMPORT io

SUB emitReport()
END SUB

SUB main()
  LET wasBuffered AS Boolean = io::isBuffered()
  io::setBuffered(TRUE)
  emitReport()
  io::setBuffered(wasBuffered)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isBuffered",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_buffered),
        }],
    });
}
