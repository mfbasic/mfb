//! `io::setBuffered` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): this member adapter reproduces its former
//! `lower_io_helper` `match` arm and hatches the finalized OS-seam body back.

use super::{adapter_app_mode, hatch_finalized};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `abi_function` body for `io::setBuffered(enabled)`. The `enabled` flag arrives
/// in argument register 0 (which the emitter reads as `return_register`); the
/// emitter toggles the thread stdout buffering flag (draining on disable), or is
/// a no-op in app mode. Hatched back pre-finalized.
pub(crate) fn lower_set_buffered(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let body = lower_io_set_buffered_helper(&symbol, adapter_app_mode(ctx))?;
    hatch_finalized(builder, body, "Nothing", "io.setBuffered")
}

const INTRO: &str = r#"Enable or disable opt-in standard-output buffering for this thread"#;
const DESC: &str = r#"`io::setBuffered` turns standard-output buffering on or off for the calling
thread and returns nothing. Buffering is **off by default**, so without this call
every `io::write` and `io::print` reaches the operating system immediately.

Passing `TRUE` only sets the enabled flag; the 4 KiB buffer itself is allocated
lazily on the first buffered write. From then on output is accumulated and issued
in blocks, collapsing a write-heavy loop from one host write per call to roughly
one per full buffer. A chunk larger than the whole buffer is written directly
after the buffer is drained, so ordering is never disturbed, and if the buffer
cannot be allocated the write falls back to going out directly — buffering is an
optimization, never a correctness dependency.

Passing `FALSE` **drains any pending bytes first** and then clears the flag, so
switching buffering off never strands output. That drain is best-effort: this
call returns `Nothing` and does not report a write failure, which instead surfaces
from the next `io::flush` or buffered write.

While buffering is on, held output is also drained when the buffer fills, on
`io::flush`, before any standard-input read — so a buffered prompt always appears
before the program blocks — and at program exit. The setting is per thread: each
thread has its own buffer and its own enabled flag, and one thread's choice is
invisible to another. Standard error is never buffered, so this call affects
standard output only. In app mode the buffer is inert and this call does nothing.
Because buffered output lives in memory until drained, a hard crash can lose bytes
that were written but not yet flushed."#;
const EX: &str = r#"Buffer a write-heavy loop and flush once at the end:

```
IMPORT io

SUB main()
  io::setBuffered(TRUE)
  MUT i AS Integer = 0
  WHILE i < 100000
    io::print(toString(i))
    i = i + 1
  END WHILE
  io::flush()
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setBuffered",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "enabled",
                desc: "`TRUE` to enable standard-output buffering for this thread; `FALSE` to drain any pending output and disable it.",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_buffered),
        }],
    });
}

// --- stdout setBuffered emitter (relocated from native/) ---

/// `io::setBuffered(enabled)` (plan-14-A §4.2): turn opt-in stdout buffering on or
/// off for this thread. Enabling just sets `OUT_ENABLED = 1` (the 4 KiB buffer is
/// allocated lazily on the first buffered write). Disabling **drains the buffer
/// first** (so pending bytes are never stranded on the off transition) and then
/// clears `OUT_ENABLED`. Returns `Nothing`. In app mode buffering is inert, so it
/// is a no-op returning OK.
pub(crate) fn lower_io_set_buffered_helper(symbol: &str, app_mode: bool) -> HelperResult {
    const FRAME_SIZE: usize = 16;
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    if !app_mode {
        let mut vregs = Vregs::new();
        let v0 = vregs.next();
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&enable),
            // Disable: drain any pending bytes first, then clear the flag. The drain
            // result is best-effort here (setBuffered returns Nothing); a real write
            // failure still surfaces on the next io::flush / buffered write.
            abi::branch_link(STDOUT_DRAIN_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
        instructions.extend([
            abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::branch(&done),
            abi::label(&enable),
            abi::move_immediate(&v0, "Integer", "1"),
            abi::store_u64(&v0, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::label(&done),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}
