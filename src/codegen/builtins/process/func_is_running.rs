//! `process::isRunning` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/isRunning.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Report whether a spawned child is still running, without blocking."#;
const DESC: &str = r#"`process::isRunning` reports whether the child behind a `Process` handle is still
alive. It performs a non-blocking check (`waitpid` with `WNOHANG` on Unix) and
returns immediately: `TRUE` while the child is running, `FALSE` once it has exited.


When the check observes that the child has just exited, it decodes and **caches**
the exit code and raw wait status in the handle, so a later `process::waitFor`
returns without blocking and `process::didSignal` can report how the child died.
Once the exit has been cached, further `isRunning` calls answer `FALSE` from the
cache without another system call.

The handle is borrowed and left open. Calling `isRunning` on a handle that has
already been dropped or detached raises `ErrResourceClosed`."#;
const EX: &str = r#"Poll a child until it finishes:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["true"])
  WHILE process::isRunning(child)
    ' still going
  END WHILE
  io::print("done")
  RETURN 0
END FUNC
```"#;

pub(crate) const IS_RUNNING: BuiltinFunction = BuiltinFunction::same(
    super::IS_RUNNING,
    "isRunning",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, "Boolean")],
)
.with_example(EX);
