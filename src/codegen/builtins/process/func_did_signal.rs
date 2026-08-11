//! `process::didSignal` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/didSignal.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

const INTRO: &str = r#"Report which signal bucket a terminated child died on."#;
const DESC: &str = r#"`process::didSignal` reports how a terminated child died, as one of the four
`Signal` buckets. It reads the raw wait status cached when the child was reaped —
by `process::waitFor` or by a `process::isRunning` that observed the exit — so it
returns `Signal.None` for a child that exited normally *or* that has not yet been
observed to terminate. Await or poll the child first if you need the death cause.



On Unix it decodes the terminating signal (`WTERMSIG`): `SIGKILL` maps to
`Signal.Kill`; the fault signals `SIGILL`, `SIGABRT`, `SIGFPE`, `SIGBUS`, and
`SIGSEGV` map to `Signal.Error`; and every other terminating signal maps to
`Signal.Terminate`. On Windows exit codes carry no signal disposition, so
`didSignal` recovers only the fault case — an NTSTATUS "error"-severity exit code
(e.g. `0xC0000005` `STATUS_ACCESS_VIOLATION`) maps to `Signal.Error`, and every
other outcome maps to `Signal.None`; this is a documented Windows limitation. The
full platform mapping is tabulated in `mfb man process types`.


Reading a handle that has already been dropped or detached raises
`ErrResourceClosed`."#;
const EX: &str = r#"Report how a child died after killing it:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "30"])
  process::signal(child, Signal.Kill)
  LET code = process::waitFor(child)
  IF process::didSignal(child) = Signal.Kill THEN
    io::print("killed")
  END IF
  RETURN 0
END FUNC
```"#;

pub(crate) const DID_SIGNAL: BuiltinFunction = BuiltinFunction::os(
    super::DID_SIGNAL,
    "didSignal",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, super::SIGNAL_TYPE)],
    lower_process_didsignal_helper_posix,
    lower_process_didsignal_helper_win,
    &["process.didSignal"],
)
.with_example(EX);

pub(crate) fn lower_process_didsignal_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_didsignal_helper(symbol, platform_imports, platform)
}

pub(crate) fn lower_process_didsignal_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::windows::lower_process_didsignal_helper(symbol, platform_imports, platform)
}
