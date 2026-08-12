//! `process::sendBytes` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/sendBytes.md`.

use std::collections::HashMap;

use crate::target::shared::code::{CodegenPlatform, HelperResult};
use crate::target::shared::registry::BuiltinFunction;

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

pub(crate) const SEND_BYTES: BuiltinFunction = BuiltinFunction::os(
    super::SEND_BYTES,
    "sendBytes",
    INTRO,
    DESC,
    &[],
    super::OV_SEND_BYTES,
    lower_process_send_helper_posix,
    lower_process_send_helper_win,
    &["process.sendBytes", "process.sendBytesTimeout"],
)
.with_example(EX);

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
