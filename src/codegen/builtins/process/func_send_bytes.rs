//! `process::sendBytes` — descriptor entry.
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
const INTRO: &str = r#"Write raw bytes to a child's standard input, with no newline added."#;
const DESC: &str = r#"`process::sendBytes` writes the raw bytes of `data` to the child's standard input,
in list order, with **no** trailing newline and no re-encoding. It is the binary
counterpart of `process::send` (which sends a `String` and appends `'\n'`); use
`sendBytes` for binary input or when you control line framing yourself.


The whole list is written before the call returns: it loops over the underlying
writes, resuming a short or interrupted write rather than treating it as complete.
An empty list writes nothing and returns immediately. Without a `timeoutMs` the
call blocks while the child's input pipe is full, waiting for room.


If the child has closed or is no longer reading its standard input — a broken pipe —
the write fails and `sendBytes` raises `ErrResourceClosed`, the same error raised
when the input was already closed with `process::close` or the handle was dropped or
detached.

`timeoutMs` bounds how long the call may wait for pipe space, in milliseconds;
on expiry it raises `ErrTimeout`. On Windows the timeout is best-effort: anonymous
pipes have no write-readiness poll, so a write to a draining reader returns at once
but a write that fills the pipe is not preempted at the deadline."#;
const EX: &str = r#"Write raw bytes to a filter and read the result:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["cat"])
  LET data AS List OF Byte = [104, 105, 10]
  process::sendBytes(child, data)
  process::close(child)
  io::print(process::receive(child))
  RETURN 0
END FUNC
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::send_bytes` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_send_bytes(
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
    // the 3-arg form is selected at codegen (`builder_values` →
    // `process.sendBytesTimeout`), and the emitter branches on the runtime-call name.
    pkg.add_function(RegistryFunction {
        name: "sendBytes",
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
                    name: "data",
                    desc: "The bytes to write, in list order, with no newline appended. An empty list writes nothing.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Byte),
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
            body: Body::abi_function_aliased(lower_send_bytes, &["sendBytesTimeout"]),
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
        true,
        call == "process.sendBytesTimeout",
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
        true,
        call == "process.sendBytesTimeout",
    )
}
