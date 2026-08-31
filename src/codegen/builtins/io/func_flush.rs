//! `io::flush` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_flush`] emits a vreg body into the
//! builder — the wrapper finalizes. Console drains the per-thread stdout buffer
//! inline; app mode appends the platform's present-on-flush GUI sequence
//! (`emit_app_io_flush`). No adapter, no pre-finalized hatch.

use super::app_unsupported;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `abi_function` body for `io::flush` (no args). App mode: append the platform
/// present-on-flush sequence. Console: the stdout-drain vreg body. The wrapper
/// finalizes.
pub(crate) fn lower_flush(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.build_mode.is_app() {
        ctx.platform
            .emit_app_io_flush(&symbol, &mut builder.instructions, &mut builder.relocations)
            .ok_or_else(|| app_unsupported(ctx.platform))??;
    } else {
        // Console: drain the per-arena MFBASIC stdout buffer via write() and report
        // a write failure — nothing else. It deliberately does NOT fsync: fsync's
        // result depends on the fd *type* (EBADF only for a genuinely closed fd,
        // benign EINVAL on pipes/char devices, 0 on a regular file), which made
        // flush's success/failure depend on the runtime environment rather than on
        // what the program actually wrote. The buffer drain's write() is the one
        // portable failure signal — identical on every platform/libc. A no-op when
        // buffering is off.
        //
        // There used to be a `stderr: bool` parameter gating this drain, on the
        // reasoning that stderr is never buffered and so has nothing to flush. No
        // caller ever passed `true` — `io::flush()` is stdout-only — so the guarded
        // and unguarded halves were the same program (bug-326-A23).
        let output_error = format!("{symbol}_output_error");
        let done = format!("{symbol}_done");
        builder
            .instructions
            .push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        builder
            .relocations
            .push(internal_branch(&symbol, STDOUT_DRAIN_SYMBOL));
        builder.instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&output_error),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&output_error),
        ]);
        raise_error_into(
            &symbol,
            "ErrWriteFailed",
            &mut builder.instructions,
            &mut builder.relocations,
        );
        builder.instructions.push(abi::label(&done));
        builder.instructions.push(abi::return_());
        builder.stack_size = 16;
    }
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "io.flush".to_string(),
    })
}

const INTRO: &str = r#"Drain the per-thread standard-output buffer"#;
const DESC: &str = r#"`io::flush` writes out any bytes currently held in this thread's MFBASIC
standard-output buffer and returns nothing. It takes no arguments.

The call is **drain-only**. It issues the pending bytes with a `write` loop and
reports whether that write succeeded; it deliberately does *not* `fsync` or
otherwise ask the host to sync standard output. The buffer drain's `write` is the
one portable failure signal, identical on every platform and libc.

It follows that `io::flush` is a **no-op when buffering is off** — the default.
Without `io::setBuffered(TRUE)` there is no MFBASIC buffer to drain, every
`io::write` and `io::print` has already reached the operating system, and this
call succeeds having done nothing. It is likewise a no-op when buffering is on
but nothing is pending.

The drain loops until the buffer is empty: a short write advances the cursor and
re-issues, and an interruption is resumed. If a write genuinely fails, the
still-unflushed bytes are slid back to the base of the buffer and kept, so a later
`io::flush` resumes from exactly where this one stopped — and this call raises
`ErrOutput`.

An explicit flush is rarely required even under buffering: the buffer is also
drained when it fills, before every standard-input read, on
`io::setBuffered(FALSE)`, and at program exit. Standard error is never buffered
and is written immediately, so it has no corresponding flush. In app mode
transcript writes are synchronous, so this call succeeds immediately."#;
const EX: &str = r#"Make buffered output visible at a checkpoint:

```
IMPORT io

SUB longRunningWork()
END SUB

SUB main()
  io::setBuffered(TRUE)
  io::print("phase one complete")
  io::flush()                ' the line reaches the terminal before the long work
  longRunningWork()
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "flush",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_flush),
        }],
    });
}
