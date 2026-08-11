//! `process::waitFor` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/waitFor.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

const INTRO: &str = r#"Block until a spawned child exits and return its exit code."#;
const DESC: &str = r#"`process::waitFor` blocks until the child behind a `Process` handle has exited, then
returns its exit code. A child that exited normally returns its exit status
(`0 .. 255` on Unix); a child killed by a signal returns `-1`.


`waitFor` is **idempotent**. The first call reaps the child (`waitpid` on Unix) and
caches its exit code and raw wait status in the handle; every later call — and a
call after `process::isRunning` already observed the exit — returns the cached code
without blocking again. Because reaping and caching happen here (or in
`isRunning`), a subsequent `process::didSignal` can report how the child died.


The handle is borrowed and left open; the child stays reaped, so letting the handle
drop afterward is a no-op rather than a second wait. Calling `waitFor` on a handle
that has already been dropped or detached raises `ErrResourceClosed`.


Standard output a child writes but the program never reads is discarded when the
pipe buffer fills, which can cause a child that keeps writing to block instead of
exiting; drain the child with `process::receive` (or close its input with
`process::close`) before `waitFor` when the child produces output."#;
const EX: &str = r#"Run a command to completion and read its exit code:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["true"])
  LET code = process::waitFor(child)
  io::print(toString(code))
  RETURN 0
END FUNC
```"#;

pub(crate) const WAIT_FOR: BuiltinFunction = BuiltinFunction::os(
    super::WAIT_FOR,
    "waitFor",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, "Integer")],
    lower_process_waitfor_helper_posix,
    lower_process_waitfor_helper_win,
    &["process.waitFor"],
)
.with_example(EX);

pub(crate) fn lower_process_waitfor_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_waitfor_helper(symbol, platform_imports, platform)
}

pub(crate) fn lower_process_waitfor_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::windows::lower_process_waitfor_helper(symbol, platform_imports, platform)
}
