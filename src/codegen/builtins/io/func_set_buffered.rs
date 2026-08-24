//! `io::setBuffered` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_set_buffered`] emits its vreg body
//! directly into the builder — the wrapper finalizes it (crypto's shape). No
//! separate emitter, no adapter, no pre-finalized hatch.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `abi_function` body for `io::setBuffered(enabled)`: the `enabled` flag arrives
/// in argument register 0; toggle the thread stdout buffering flag (draining on
/// disable), or no-op in app mode. Emits its vreg stream into `builder`; the
/// wrapper finalizes.
pub(crate) fn lower_set_buffered(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    const FRAME_SIZE: usize = 16;
    let symbol = builder.current_symbol.clone();
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    if !ctx.build_mode.is_app() {
        let mut vregs = Vregs::new();
        let v0 = vregs.next();
        builder.instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&enable),
            abi::branch_link(STDOUT_DRAIN_SYMBOL),
        ]);
        builder
            .relocations
            .push(internal_branch(&symbol, STDOUT_DRAIN_SYMBOL));
        builder.instructions.extend([
            abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::branch(&done),
            abi::label(&enable),
            abi::move_immediate(&v0, "Integer", "1"),
            abi::store_u64(&v0, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::label(&done),
        ]);
    }
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    builder.stack_size = FRAME_SIZE;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "io.setBuffered".to_string(),
    })
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
