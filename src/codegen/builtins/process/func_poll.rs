//! `process::poll` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/poll.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

const INTRO: &str = r#"Test whether a child's output stream is readable within a timeout."#;
const DESC: &str = r#"`process::poll` reports whether a following read of a child's output stream can
proceed without blocking. It returns `TRUE` when the selected stream is readable —
**including** the case where the child has closed it and the stream is at end of
output, so a draining `process::receive`/`process::receiveBytes` can follow — and
`FALSE` when nothing became readable before the deadline. The stream is inspected
only; no data is consumed, so a `TRUE` result leaves the bytes in place for the next
read.

`ms` is the wait bound in milliseconds. `0` is a non-blocking check that returns the
stream's current readiness immediately; a positive value waits up to that long; a
timeout that elapses with nothing readable returns `FALSE` (poll reports readiness
as a boolean and never raises `ErrTimeout`).


Without a `from` argument `poll` inspects the child's standard output; pass a
`Stream` value to choose standard output or standard error."#;
const EX: &str = r#"Read a line only if one is ready within 100 ms:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  IF process::poll(child, 100) THEN
    io::print(process::receive(child))
  END IF
  RETURN 0
END FUNC
```

Check the child's standard error without blocking:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo oops 1>&2")
  IF process::poll(sh, 0, Stream.StdErr) THEN
    io::print(process::receive(sh, Stream.StdErr))
  END IF
  RETURN 0
END FUNC
```"#;

pub(crate) const POLL: BuiltinFunction = BuiltinFunction::os(
    super::POLL,
    "poll",
    INTRO,
    DESC,
    &[],
    super::OV_POLL,
    lower_process_poll_helper_posix,
    lower_process_poll_helper_win,
    &["process.poll", "process.pollFrom"],
)
.with_example(EX);

pub(crate) fn lower_process_poll_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_poll_helper(
        symbol,
        platform_imports,
        platform,
        call == "process.pollFrom",
    )
}

pub(crate) fn lower_process_poll_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::windows::lower_process_poll_helper(
        symbol,
        platform_imports,
        platform,
        call == "process.pollFrom",
    )
}
