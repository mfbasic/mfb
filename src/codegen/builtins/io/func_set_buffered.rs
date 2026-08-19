//! `io::setBuffered` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`crate::codegen::builtins::io::native::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

use crate::codegen::builtins::io::native::{
    adapter_app_mode, hatch_finalized, lower_io_set_buffered_helper,
};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
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
