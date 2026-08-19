//! `io::flush` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): this member adapter reproduces its former
//! `lower_io_helper` `match` arm and hatches the finalized OS-seam body back.

use crate::codegen::builtins::io::native::{
    adapter_app_mode, app_unsupported, hatch_finalized, lower_io_flush_helper,
};
use crate::codegen::engine::builder::{pad_no_slots, CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `abi_function` body for `io::flush` (no args). Console: drain the per-thread
/// stdout buffer (`lower_io_flush_helper`). App mode: synchronous transcript
/// writes make flush an immediate success (`emit_app_io_flush_helper`). Hatched
/// back pre-finalized.
pub(crate) fn lower_flush(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let body = if adapter_app_mode(ctx) {
        pad_no_slots(
            ctx.platform
                .emit_app_io_flush_helper(&symbol)
                .ok_or_else(|| app_unsupported(ctx.platform))??,
        )
    } else {
        lower_io_flush_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
    hatch_finalized(builder, body, "Nothing", "io.flush")
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
re-issues, and an `EINTR` interruption retries. If a write genuinely fails, the
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
