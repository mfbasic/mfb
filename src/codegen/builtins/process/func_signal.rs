//! `process::signal` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/signal.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Deliver a cross-platform signal bucket to a child process."#;
const DESC: &str = r#"`process::signal` delivers one of the four `Signal` buckets to the child behind a
`Process` handle. The bucket abstracts over platform signal numbers so the same
call works on Unix and Windows. `Signal.None` is a no-op. On Unix, `Signal.Kill`
sends `SIGKILL`, `Signal.Terminate` sends `SIGTERM`, and `Signal.Error` sends
`SIGABRT`.


On Windows there is no way to deliver an arbitrary signal to a child without a
shared console, so every terminating bucket maps to the same best-effort
`TerminateProcess`, with a POSIX-flavored exit code (`128 + signo`, so `137`/`143`/
`134` for `Kill`/`Terminate`/`Error`) that a later `process::waitFor` can read back;
there is no per-signal fidelity. The full platform mapping is tabulated in
`mfb man process types`.

Delivery does not wait for or reap the child; call `process::waitFor` afterward to
collect the exit status, or `process::didSignal` to read back which bucket a
terminated child died on. Signalling a handle that has already been dropped or
detached raises `ErrResourceClosed`."#;
const EX: &str = r#"Ask a long-running child to stop, then wait for it:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "30"])
  process::signal(child, Signal.Terminate)
  io::print(toString(process::waitFor(child)))
  RETURN 0
END FUNC
```"#;

pub(crate) const SIGNAL: BuiltinFunction = BuiltinFunction::same(
    super::SIGNAL,
    "signal",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_SIGNAL, "Nothing")],
)
.with_example(EX);
