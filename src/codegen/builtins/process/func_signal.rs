//! `process::signal` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/signal.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

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

pub(crate) const SIGNAL: BuiltinFunction = BuiltinFunction::os(
    super::SIGNAL,
    "signal",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_SIGNAL, "Nothing")],
    lower_process_signal_helper_posix,
    lower_process_signal_helper_win,
    &["process.signal"],
)
.with_example(EX);

pub(crate) fn lower_process_signal_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_signal_helper(symbol, platform_imports, platform)
}

pub(crate) fn lower_process_signal_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::windows::lower_process_signal_helper(symbol, platform_imports, platform)
}
