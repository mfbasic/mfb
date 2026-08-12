//! `process::pid` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/pid.md`.

use std::collections::HashMap;

use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;
use crate::target::shared::registry::BuiltinFunction;

use super::native::*;

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

pub(crate) const PID: BuiltinFunction = BuiltinFunction::os(
    super::PID,
    "pid",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, "Integer")],
    lower_process_pid_helper_posix,
    lower_process_pid_helper_win,
    &["process.pid"],
)
.with_example(EX);

pub(crate) fn lower_process_pid_helper_posix(
    _call: &str,
    symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let pid = v.next();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&pid, &file, RESOURCE_OFFSET_HANDLE),
        abi::move_register(RESULT_VALUE_REGISTER, &pid),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ];
    let mut relocations = Vec::new();
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(crate) fn lower_process_pid_helper_win(
    _call: &str,
    symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(RESULT_VALUE_REGISTER, &file, PROC_STATUS),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ];
    let mut relocations = Vec::new();
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
