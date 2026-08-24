//! `io::isBuffered` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_is_buffered`] emits its vreg body
//! directly into the builder — the wrapper finalizes it (crypto's shape). No
//! separate emitter, no adapter, no pre-finalized hatch.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `abi_function` body for `io::isBuffered` (no args): report the thread stdout
/// buffering flag (`OUT_ENABLED != 0`), or `FALSE` in app mode where the buffer is
/// inert. Emits its vreg stream into `builder`; the wrapper finalizes.
pub(crate) fn lower_is_buffered(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    const FRAME_SIZE: usize = 16;
    let symbol = builder.current_symbol.clone();
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");
    if ctx.build_mode.is_app() {
        builder
            .instructions
            .push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    } else {
        let mut vregs = Vregs::new();
        let v0 = vregs.next();
        builder.instructions.extend([
            abi::load_u64(&v0, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate(&v0, "0"),
            abi::branch_ne(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        ]);
    }
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    builder.stack_size = FRAME_SIZE;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: "io.isBuffered".to_string(),
    })
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
