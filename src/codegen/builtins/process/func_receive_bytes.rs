//! `process::receiveBytes` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/receiveBytes.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

const INTRO: &str = r#"Read one available chunk of raw bytes from a child's output."#;
const DESC: &str = r#"`process::receiveBytes` reads the next available chunk of raw bytes from a child's
output stream and returns it as a `List OF Byte`. It performs one underlying read,
so it returns as soon as any data is available rather than waiting to fill a fixed
size, and the returned list is frequently shorter than the amount the child will
eventually produce. It does no line framing, decoding, or newline translation, so
it is the right call for binary output; use `process::receive` for text lines.


Without a `from` argument it reads the child's standard output; pass a `Stream`
value to choose standard output or standard error. The call blocks until at least
one byte is available or the stream ends. A pipe read returns any buffered bytes
before signalling end of stream, so late output is drained; only a read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. On success the
result always holds at least one byte — end of output is never an empty list."#;
const EX: &str = r#"Read one chunk of raw output and report its length:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  LET chunk = process::receiveBytes(child)
  io::print(toString(len(chunk)))
  RETURN 0
END FUNC
```"#;

pub(crate) const RECEIVE_BYTES: BuiltinFunction = BuiltinFunction::os(
    super::RECEIVE_BYTES,
    "receiveBytes",
    INTRO,
    DESC,
    &[],
    super::OV_RECEIVE_BYTES,
    lower_process_receivebytes_helper_posix,
    lower_process_receivebytes_helper_win,
    &["process.receiveBytes", "process.receiveBytesFrom"],
)
.with_example(EX);

pub(crate) fn lower_process_receivebytes_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_receivebytes_helper(
        symbol,
        platform_imports,
        platform,
        call == "process.receiveBytesFrom",
    )
}

pub(crate) fn lower_process_receivebytes_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::windows::lower_process_receivebytes_helper(
        symbol,
        platform_imports,
        platform,
        call == "process.receiveBytesFrom",
    )
}
