//! `process::isRunning` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor and those entry fns.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;

use crate::codegen::error::emission::emit_fail;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_shared::*;
use super::gen_unix::*;
const INTRO: &str = r#"Report whether a spawned child is still running, without blocking."#;
const DESC: &str = r#"`process::isRunning` reports whether the child behind a `Process` handle is still
alive. It performs a non-blocking check (`waitpid` with `WNOHANG` on Unix) and
returns immediately: `TRUE` while the child is running, `FALSE` once it has exited.


When the check observes that the child has just exited, it decodes and **caches**
the exit code and raw wait status in the handle, so a later `process::waitFor`
returns without blocking and `process::didSignal` can report how the child died.
Once the exit has been cached, further `isRunning` calls answer `FALSE` from the
cache without asking the operating system again.

The handle stays open. Calling `isRunning` on a handle that has
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

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::is_running` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_is_running(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.platform.family()
        == crate::codegen::engine::types::PlatformFamily::Windows
    {
        lower_process_isrunning_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    } else {
        lower_process_isrunning_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isRunning",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "p",
                desc: "The child process handle. The handle stays open — you still close it. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::named(super::PROCESS_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_running),
        }],
    });
}

pub(crate) fn lower_process_isrunning_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    const STATUS_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let reaped = v.next();
    let ret = v.next();
    let status = v.next();
    let exit = v.next();
    let one = v.next();
    let s0 = v.next();
    let s1 = v.next();
    let closed_l = format!("{symbol}_closed");
    let running = format!("{symbol}_running");
    let not_running = format!("{symbol}_not_running");
    let ret_false = format!("{symbol}_ret_false");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&ret_false),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STATUS_SLOT),
        abi::move_immediate(abi::c_arg(2), "Integer", WNOHANG),
    ];
    let mut relocations = Vec::new();
    platform.emit_external_call(
        "waitpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(&ret, abi::c_return(0)),
        // 0 -> running; >0 -> reaped now; <0 -> ECHILD (not running, nothing to cache).
        abi::compare_immediate(&ret, "0"),
        abi::branch_gt(&not_running),
        abi::branch_lt(&ret_false),
        abi::label(&running),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // Reaped just now: decode + cache, then return false.
        abi::label(&not_running),
        abi::load_u32(&status, abi::stack_pointer(), STATUS_SLOT),
    ]);
    emit_decode_status(&status, &exit, &s0, &s1, symbol, &mut instructions);
    instructions.extend([
        abi::store_u64(&status, &file, PROC_STATUS),
        abi::store_u64(&exit, &file, PROC_EXITCODE),
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, PROC_REAPED),
        abi::label(&ret_false),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
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
    Ok((instructions, relocations, 16))
}

pub(crate) fn lower_process_isrunning_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    const EXIT: usize = 0x20;
    const FILE: usize = 0x28;
    const FRAME: usize = 0x30;
    const STILL_ACTIVE: &str = "259";
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let ret_false = format!("{symbol}_ret_false");
    let reaped_now = format!("{symbol}_reaped_now");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_REAPED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&ret_false),
        // GetExitCodeProcess(hProcess, &exit)
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::mfb_arg(1), sp, EXIT),
    ];
    platform.emit_external_call(
        "GetExitCodeProcess",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u32(abi::mfb_arg(0), sp, EXIT),
        abi::compare_immediate(abi::mfb_arg(0), STILL_ACTIVE),
        abi::branch_ne(&reaped_now),
        // Still running.
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // Exited: cache the raw code and return false.
        abi::label(&reaped_now),
        abi::load_u64(abi::mfb_arg(1), sp, FILE),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), PROC_STATUS),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), PROC_EXITCODE),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), PROC_REAPED),
        abi::label(&ret_false),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
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
    Ok((instructions, relocations, 0))
}
