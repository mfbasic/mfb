//! `process::detach` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;

use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_shared::*;
const INTRO: &str = r#"Let a child keep running after the program exits."#;
const DESC: &str = r#"`process::detach` lets a child keep running **without** killing it. It
closes the parent-side pipe ends, arranges for the operating system to clean the
child up when it eventually exits (on Unix a dedicated thread waits for that one
child, so no zombie is left behind), and marks the handle closed. The child keeps
running on its own and survives the parent's exit.

Detaching one child affects only that child. Every other `process::Process` keeps
its own behavior — in particular `process::waitFor` on a handle you did not detach
still reports that child's real exit code.

This is the counterpart to the default drop behavior. Normally letting a `Process`
go out of scope force-kills and reaps the child; `detach` is the deliberate opt-out
for a child that should outlive the program — a daemon, a background job, a handoff
to another process.

Because `detach` marks the handle closed, it ends the handle for all practical
purposes: every later `process::` call on it — including a second `detach` — raises
`ErrResourceClosed`, and the eventual end of the binding is a no-op rather than a kill."#;
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

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::detach` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_detach(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.platform.family()
        == crate::codegen::engine::types::PlatformFamily::Windows
    {
        lower_process_detach_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    } else {
        lower_process_detach_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "detach",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "p",
                desc: "The child process handle to release. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::named(super::PROCESS_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_detach),
        }],
    });
}

pub(crate) fn lower_process_detach_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    // The `pthread_t` `pthread_create` fills in, in this body's sp-relative locals.
    const TID_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let fd = v.next();
    let one = v.next();
    let pid = v.next();
    let closed_l = format!("{symbol}_closed");
    let no_reaper = format!("{symbol}_no_reaper");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&fd, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&fd, "0"),
        abi::branch_ne(&closed_l),
    ];
    let mut relocations = Vec::new();
    for off in [PROC_STDIN_W, PROC_STDOUT_R, PROC_STDERR_R] {
        let skip = format!("{symbol}_skip_{off}");
        instructions.extend([
            abi::load_u64(&fd, &file, off),
            abi::compare_immediate(&fd, "0"),
            abi::branch_lt(&skip),
            abi::move_register(abi::c_arg(0), &fd),
        ]);
        platform.emit_external_call(
            "close",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::label(&skip));
    }
    // Reap this ONE child on a dedicated thread (bug-474):
    //   pthread_create(&tid, NULL, _mfb_rt_process_reaper, (void *)pid);
    //   if (rc == 0) pthread_detach(tid);
    //
    // This replaced `signal(SIGCHLD, SIG_IGN)`, which asked the kernel to auto-reap
    // EVERY child of the program, not just this one: after any `detach`, a `waitpid`
    // on an unrelated child failed with `ECHILD` and `process::waitFor` reported the
    // never-written cached exit code `0` instead of the child's real status. The
    // process-wide disposition is now left exactly as the program found it.
    //
    // The pid is passed BY VALUE — the reaper must not hold the `Process` record,
    // whose arena block is reclaimed when the detaching scope exits while the thread
    // is still blocked in `waitpid`. `tid` is pre-zeroed and `pthread_create`'s
    // return checked, so a failed create never hands garbage to `pthread_detach`
    // (the child is then left for the program's exit to reparent, never killed).
    //
    // One thread per detached child, so the thread count tracks the number of *live*
    // detached children — each reaper exits the moment its child does. That is the
    // price of a per-child reap: the alternatives (a `SIGCHLD` handler, or a swept
    // pid list) both need a process-wide signal disposition or a fixed-capacity
    // global table, which is what made this a whole-program defect in the first
    // place. The threads are cheap (default stack, no arena, one blocking libc call
    // — see `gen_unix::lower_process_reaper_helper`) and the failure mode is
    // graceful: at the thread limit `pthread_create` returns `EAGAIN`, the `b.ne`
    // below skips the detach, and the child is simply left for the program's exit to
    // reparent. `detach` still succeeds and no other child's exit status is touched.
    //
    // Skip the whole thing when the child has ALREADY been reaped (a `waitFor` or
    // `isRunning` before the `detach` sets `PROC_REAPED`). There is nothing left to
    // wait for, and starting a reaper on a dead pid is not merely wasteful: the
    // kernel may already have recycled that pid onto a LATER child of ours, and the
    // reaper would then consume that child's exit status — bug-474 in miniature.
    instructions.extend([
        abi::load_u64(&fd, &file, PROC_REAPED),
        abi::compare_immediate(&fd, "0"),
        abi::branch_ne(&no_reaper),
        abi::load_u64(&pid, &file, RESOURCE_OFFSET_HANDLE),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TID_SLOT),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), TID_SLOT),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    push_symbol_address(
        symbol,
        PROCESS_REAPER_SYMBOL,
        abi::c_arg(2),
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::c_arg(3), &pid));
    platform.emit_external_call(
        "pthread_create",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&no_reaper),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), TID_SLOT),
    ]);
    platform.emit_external_call(
        "pthread_detach",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&no_reaper));
    instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, RESOURCE_OFFSET_CLOSED),
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

pub(crate) fn lower_process_detach_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ];
    for off in [
        PROC_STDIN_W,
        PROC_STDOUT_R,
        PROC_STDERR_R,
        RESOURCE_OFFSET_HANDLE,
    ] {
        let skip = format!("{symbol}_skip_{off}");
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), sp, FILE),
            abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), off),
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_lt(&skip), // -1 sentinel (already closed) — skip
        ]);
        platform.emit_external_call(
            "CloseHandle",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::label(&skip));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
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
