//! `process::detach` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/detach.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

const INTRO: &str =
    r#"Relinquish ownership of a child so it keeps running after the program exits."#;
const DESC: &str = r#"`process::detach` relinquishes ownership of a child **without** killing it. It
closes the parent-side pipe ends, arranges for the operating system to auto-reap the
child when it eventually exits (on Unix, by setting `SIGCHLD` to be ignored so the
kernel reaps it and no zombie is left), and marks the handle closed. The child keeps
running on its own and survives the parent's exit.

This is the counterpart to the default drop behavior. Normally letting a `Process`
go out of scope force-kills and reaps the child; `detach` is the deliberate opt-out
for a child that should outlive the program — a daemon, a background job, a handoff
to another process.

Because `detach` marks the handle closed, it consumes the handle for all practical
purposes: every later `process::` call on it — including a second `detach` — raises
`ErrResourceClosed`, and the eventual scope-drop is a no-op rather than a kill."#;
const EX: &str = r#"Start a background job and let it outlive the program:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES job = process::shell("sleep 5")
  process::detach(job)
  io::print("job detached")
  RETURN 0
END FUNC
```"#;

pub(crate) const DETACH: BuiltinFunction = BuiltinFunction::os(
    super::DETACH,
    "detach",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, "Nothing")],
    lower_process_detach_helper_posix,
    lower_process_detach_helper_win,
    &["process.detach"],
)
.with_example(EX);

pub(crate) fn lower_process_detach_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::unix::lower_process_detach_helper(symbol, platform_imports, platform)
}

pub(crate) fn lower_process_detach_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    super::native::windows::lower_process_detach_helper(symbol, platform_imports, platform)
}
