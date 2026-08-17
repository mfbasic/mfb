//! `process::waitFor` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/waitFor.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;
use crate::types::ParameterType;

use super::native::unix::*;
use super::native::*;

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

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "waitFor",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "p",
                desc: "The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::Named(super::PROCESS_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(
                Some(lower_process_waitfor_helper_posix),
                Some(lower_process_waitfor_helper_win),
                None,
            ),
        }],
    });
}

pub(crate) fn lower_process_waitfor_helper_posix(
    _call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const STATUS_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let reaped = v.next();
    let status = v.next();
    let exit = v.next();
    let one = v.next();
    let s0 = v.next();
    let s1 = v.next();
    let closed_l = format!("{symbol}_closed");
    let cached = format!("{symbol}_cached");
    let echild = format!("{symbol}_echild");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&cached),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STATUS_SLOT),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "waitpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&echild),
        abi::load_u32(&status, abi::stack_pointer(), STATUS_SLOT),
    ]);
    emit_decode_status(&status, &exit, &s0, &s1, symbol, &mut instructions);
    instructions.extend([
        abi::store_u64(&status, &file, PROC_STATUS),
        abi::store_u64(&exit, &file, PROC_EXITCODE),
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, PROC_REAPED),
        abi::move_register(RESULT_VALUE_REGISTER, &exit),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // ECHILD: mark reaped, return cached (default 0).
        abi::label(&echild),
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, PROC_REAPED),
        abi::label(&cached),
        abi::load_u64(RESULT_VALUE_REGISTER, &file, PROC_EXITCODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(crate) fn lower_process_waitfor_helper_win(
    _call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Explicit Win64 frame (depth-1, no vregs): shadow [0x00..0x20), then EXIT (the
    // `GetExitCodeProcess` out-param) and FILE (the record pointer, live across the
    // two kernel32 calls). Reserving the shadow is mandatory — a callee writes its
    // 32-byte shadow into the caller's [sp, sp+0x20), which would otherwise clobber
    // these slots (`call_external` does not reserve it).
    const EXIT: usize = 0x20;
    const FILE: usize = 0x28;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let cached = format!("{symbol}_cached");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_REAPED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&cached),
        // WaitForSingleObject(hProcess, INFINITE)
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "4294967295"),
    ];
    platform.emit_libc_call(
        "WaitForSingleObject",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // GetExitCodeProcess(hProcess, &exit)
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::mfb_arg(1), sp, EXIT),
    ]);
    platform.emit_libc_call(
        "GetExitCodeProcess",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u32(abi::mfb_arg(0), sp, EXIT),
        abi::load_u64(abi::mfb_arg(2), sp, FILE),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(2), PROC_STATUS), // raw code (didSignal)
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(2), PROC_EXITCODE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(2), PROC_REAPED),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_arg(0)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&cached),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::mfb_arg(0), PROC_EXITCODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
