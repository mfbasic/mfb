//! `process::sendBytes` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/sendBytes.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodegenPlatform, HelperResult};
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

pub(super) fn register(pkg: &mut RegistryPackage) {
    // The optional trailing `timeoutMs` widens arity to 3 and is NOT default-padded:
    // the 3-arg form is selected at codegen (`builder_values` →
    // `process.sendBytesTimeout`), and the emitter branches on the runtime-call name.
    pkg.add_function(RegistryFunction {
        name: "sendBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "p",
                    desc: "The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`.",
                    aliases: &["process"],
                    ty: ParameterType::Named(super::PROCESS_TYPE),
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
            body: Body::native_os_seam(
                Some(lower_process_send_helper_posix),
                Some(lower_process_send_helper_win),
                &["sendBytesTimeout"],
            ),
        }],
    });
}

pub(crate) fn lower_process_send_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_send_helper(
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
) -> HelperResult {
    super::native::windows::lower_process_send_helper(
        symbol,
        platform_imports,
        platform,
        true,
        call == "process.sendBytesTimeout",
    )
}
