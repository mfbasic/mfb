//! `process::send` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor and those entry fns.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use std::collections::HashMap;

use super::gen_shared::ProcBodyParts;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTRO: &str = r#"Write a line of text to a child's standard input, appending a newline."#;
const DESC: &str = r#"`process::send` writes the UTF-8 bytes of `text` to the child's standard input and
then appends a single newline (`'\n'`), so each call delivers one complete line to
a line-oriented child. To write raw bytes with no trailing newline, use
`process::sendBytes`.

The whole payload is written before the call returns: it loops over the underlying
writes, advancing past whatever each accepted and retrying an interrupted write, so
a short write is resumed rather than mistaken for completion. Without a `timeoutMs`
the call blocks while the child's input pipe is full, waiting for the child to
consume enough to make room.

If the child has closed or is no longer reading its standard input — a broken pipe —
the write fails and `send` raises `ErrResourceClosed`, the same error raised when
the input was already closed with `process::close` or the handle was dropped or
detached.

`timeoutMs` bounds how long the call may wait for pipe space, in milliseconds;
when the deadline passes with the payload not fully written it raises `ErrTimeout`.
On Windows the timeout is best-effort: anonymous pipes have no write-readiness poll,
so a write to a draining reader returns immediately (the common case) but a write
that fills the pipe is not preempted at the deadline."#;
const EX: &str = r#"Send two lines to a filter and read its sorted output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sorter = process::spawn(["sort"])
  process::send(sorter, "banana")
  process::send(sorter, "apple")
  process::close(sorter)
  io::print(process::receive(sorter))
  RETURN 0
END FUNC
```

Bound the write with a one-second timeout:

```
IMPORT process

FUNC main AS Integer
  RES child = process::spawn(["cat"])
  process::send(child, "hello", 1000)
  RETURN 0
END FUNC
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::send` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_send(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        if ctx.platform.family() == crate::codegen::engine::types::PlatformFamily::Windows {
            lower_process_send_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        } else {
            lower_process_send_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // The optional trailing `timeoutMs` widens arity to 3 and is NOT default-padded:
    // the 3-arg form is selected at codegen (`builder_values` → `process.sendTimeout`),
    // and the emitter branches on the runtime-call name.
    pkg.add_function(RegistryFunction {
        name: "send",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "p",
                    desc: "The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`.",
                    aliases: &["process"],
                    ty: ParameterType::Named(super::PROCESS_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "text",
                    desc: "The text to write to the child's standard input; a single newline is appended.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "timeoutMs",
                    desc: "Optional. The maximum time to wait for room in the child's input pipe, in milliseconds; on expiry the call raises `ErrTimeout`. Best-effort on Windows.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function_aliased(lower_send, &["sendTimeout"]),
        }],
    });
}

pub(crate) fn lower_process_send_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    super::gen_unix::lower_process_send_helper(
        symbol,
        platform_imports,
        platform,
        false,
        call == "process.sendTimeout",
    )
}

pub(crate) fn lower_process_send_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    super::gen_windows::lower_process_send_helper(
        symbol,
        platform_imports,
        platform,
        false,
        call == "process.sendTimeout",
    )
}
