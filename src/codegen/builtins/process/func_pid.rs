//! `process::pid` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/pid.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Return the operating-system process ID of a spawned child."#;
const DESC: &str = r#"`process::pid` reads the operating-system process identifier of the child behind a
`Process` handle. The value is the child pid captured when the process was spawned
and cached in the handle record, so `pid` performs no system call and never blocks;
it returns the same value for the life of the handle, even after the child has
exited (the pid is not re-checked for liveness — use `process::isRunning` for
that).

The handle is borrowed and left open. Calling `pid` on a handle that has already
been dropped or detached raises `ErrResourceClosed`."#;
const EX: &str = r#"Print the child's process ID:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "1"])
  io::print(toString(process::pid(child)))
  RETURN 0
END FUNC
```"#;

pub(crate) const PID: BuiltinFunction = BuiltinFunction::same(
    super::PID,
    "pid",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, "Integer")],
)
.with_example(EX);
