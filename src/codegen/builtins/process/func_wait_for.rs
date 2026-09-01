//! `process::waitFor` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{emit_alloc, emit_arena_free};
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
const INTRO: &str = r#"Block until a spawned child exits and return its exit code."#;
const DESC: &str = r#"`process::waitFor` blocks until the child behind a `Process` handle has exited, then
returns its exit code. A child that exited normally returns its exit status
(`0 .. 255` on Unix); a child killed by a signal returns `-1`.


**A `process::detach` anywhere in the program breaks the exit code.** On Unix,
`process::detach` asks the operating system to clean up finished children
automatically, and that setting applies to the whole program rather than to one
child: after any `detach`, `waitFor` on any *other* handle reports `0` instead
of that child's real exit code. Do not detach a child while you still need an
accurate exit status from another one.

`waitFor` is **idempotent**. The first call reaps the child (`waitpid` on Unix) and
caches its exit code and raw wait status in the handle; every later call — and a
call after `process::isRunning` already observed the exit — returns the cached code
without blocking again. Because reaping and caching happen here (or in
`isRunning`), a subsequent `process::didSignal` can report how the child died.


The handle stays open and the child stays reaped, so ending the binding
afterwards does not wait a second time. Calling `waitFor` on a handle whose
binding has already ended, or that has been detached, raises
`ErrResourceClosed`.


**A chatty child does not stall the wait.** While it waits, `waitFor` keeps
reading the child's standard output and standard error, so a child that writes
more than the pipe between you can hold does not get stuck part-way through its
own write. What `waitFor` reads is kept, not thrown away: `process::receive` and
`process::receiveBytes` hand those bytes back afterwards, in order, ahead of
anything still in the pipe. You do not have to read a child dry before waiting
for it.


`waitFor` will hold up to 16 MiB of each stream this way. A child that writes
more than that before it exits raises `ErrResourceBusy` instead of waiting on —
everything read so far is still there for you to read, and a later `waitFor`,
once you have taken some of it back, picks up where this one stopped."#;
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

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::wait_for` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_wait_for(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.platform.family()
        == crate::codegen::engine::types::PlatformFamily::Windows
    {
        lower_process_waitfor_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    } else {
        lower_process_waitfor_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
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
                desc: "The child process handle. The handle stays open — you still close it. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::named(super::PROCESS_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_wait_for),
        }],
    });
}

/// The Unix `waitFor` body: a drain-while-you-wait loop, not a bare `waitpid`.
///
/// bug-475: the old body called blocking `waitpid` and nothing else. The parent
/// held the child's stdout/stderr read ends open and never read them, so a child
/// writing more than a pipeful (16–64 KiB) blocked in its own `write` and could
/// never exit, while the parent blocked in `waitpid` for a child that could never
/// finish. The loop below reaps with `WNOHANG` and, whenever the child is still
/// alive, `poll`s the two read ends and moves whatever is there into the
/// per-stream spill buffer (`gen_shared`'s block on record slots 80/88) that
/// `receive`/`receiveBytes`/`poll` serve from. Draining rather than discarding is
/// the whole point: the bytes are still the caller's to read afterwards.
///
/// Two shapes are preserved deliberately. A child that has *already* exited is
/// reaped by the very first `WNOHANG` call, so the common quick-child case makes
/// no `poll` and allocates nothing. And when both streams have reached EOF the
/// loop drops into a *blocking* `waitpid` — nothing can deadlock on our pipes any
/// more, and a grandchild holding the pipe open no longer keeps us spinning.
pub(crate) fn lower_process_waitfor_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    const STATUS_SLOT: usize = 0;
    // Two `struct pollfd { int fd; short events; short revents; }` — 8 bytes each,
    // the same layout `process::poll` builds.
    const PFD_SLOT: usize = 8;
    const POLLIN: &str = "1";
    // The poll only ever times out while the child is alive *and* quiet: its exit
    // closes the write ends, which wakes `poll` immediately. So this bounds only
    // the rare grandchild-holds-the-pipe case, not ordinary exit latency.
    const POLL_TIMEOUT_MS: &str = "200";
    const EINTR: &str = "4";

    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let reaped = v.next();
    let status = v.next();
    let exit = v.next();
    let one = v.next();
    let s0 = v.next();
    let s1 = v.next();
    let fd_out = v.next();
    let fd_err = v.next();
    let eof_out = v.next();
    let eof_err = v.next();
    let cur_fd = v.next();
    let cur_slot = v.next();
    let cur_is_err = v.next();
    let buf = v.next();
    let cap = v.next();
    let len = v.next();
    let want = v.next();
    let newcap = v.next();
    let size = v.next();
    let errno = v.next();
    let t0 = v.next();
    let t1 = v.next();
    let t2 = v.next();

    let closed_l = format!("{symbol}_closed");
    let cached = format!("{symbol}_cached");
    let echild = format!("{symbol}_echild");
    let done = format!("{symbol}_done");
    let out_open = format!("{symbol}_out_open");
    let err_open = format!("{symbol}_err_open");
    let wait_loop = format!("{symbol}_wait_loop");
    let have_status = format!("{symbol}_have_status");
    let poll_streams = format!("{symbol}_poll_streams");
    let block_wait = format!("{symbol}_block_wait");
    let pfd0_set = format!("{symbol}_pfd0_set");
    let pfd1_set = format!("{symbol}_pfd1_set");
    let check_stderr = format!("{symbol}_check_stderr");
    let append = format!("{symbol}_append");
    let append_have = format!("{symbol}_append_have");
    let grow_loop = format!("{symbol}_grow_loop");
    let do_grow = format!("{symbol}_do_grow");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let cap_ready = format!("{symbol}_cap_ready");
    let size_ok = format!("{symbol}_size_ok");
    let read_now = format!("{symbol}_read_now");
    let read_err = format!("{symbol}_read_err");
    let mark_eof = format!("{symbol}_mark_eof");
    let mark_eof_err = format!("{symbol}_mark_eof_err");
    let over_limit = format!("{symbol}_over_limit");
    let alloc_fail = format!("{symbol}_alloc_fail");

    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&cached),
        // Cache the two read ends and seed their EOF flags. A stream already
        // marked closed (fd < 0) is finished before we start.
        abi::load_u64(&fd_out, &file, PROC_STDOUT_R),
        abi::move_immediate(&eof_out, "Integer", "0"),
        abi::compare_immediate(&fd_out, "0"),
        abi::branch_ge(&out_open),
        abi::move_immediate(&eof_out, "Integer", "1"),
        abi::label(&out_open),
        abi::load_u64(&fd_err, &file, PROC_STDERR_R),
        abi::move_immediate(&eof_err, "Integer", "0"),
        abi::compare_immediate(&fd_err, "0"),
        abi::branch_ge(&err_open),
        abi::move_immediate(&eof_err, "Integer", "1"),
        abi::label(&err_open),
        // --- drain-while-you-wait -------------------------------------------
        abi::label(&wait_loop),
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
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&echild),
        abi::branch_gt(&have_status),
        // Still running. If either stream can still deliver bytes, service it;
        // once both are at EOF a blocking wait is safe.
        abi::compare_immediate(&eof_out, "0"),
        abi::branch_eq(&poll_streams),
        abi::compare_immediate(&eof_err, "0"),
        abi::branch_eq(&poll_streams),
        abi::label(&block_wait),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STATUS_SLOT),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "waitpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_le(&echild),
        abi::branch(&have_status),
        // --- poll both read ends --------------------------------------------
        // A finished stream goes in as fd -1, which `poll` ignores (revents 0),
        // so the array is always two entries wide.
        abi::label(&poll_streams),
        abi::move_register(&t0, &fd_out),
        abi::compare_immediate(&eof_out, "0"),
        abi::branch_eq(&pfd0_set),
        abi::bitwise_not(&t0, abi::ZERO),
        abi::label(&pfd0_set),
        abi::store_u32(&t0, abi::stack_pointer(), PFD_SLOT),
        abi::move_immediate(&t1, "Integer", POLLIN),
        abi::store_u16(&t1, abi::stack_pointer(), PFD_SLOT + 4),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), PFD_SLOT + 6),
        abi::move_register(&t0, &fd_err),
        abi::compare_immediate(&eof_err, "0"),
        abi::branch_eq(&pfd1_set),
        abi::bitwise_not(&t0, abi::ZERO),
        abi::label(&pfd1_set),
        abi::store_u32(&t0, abi::stack_pointer(), PFD_SLOT + 8),
        abi::move_immediate(&t1, "Integer", POLLIN),
        abi::store_u16(&t1, abi::stack_pointer(), PFD_SLOT + 12),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), PFD_SLOT + 14),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), PFD_SLOT),
        abi::move_immediate(abi::c_arg(1), "Integer", "2"),
        abi::move_immediate(abi::c_arg(2), "Integer", POLL_TIMEOUT_MS),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        // Timed out, or was interrupted: re-check the child and poll again.
        abi::branch_le(&wait_loop),
        // Service one ready stream per pass — stdout first — then re-check the
        // child. Two streams sharing one append block is what keeps this body
        // from needing a second copy of it.
        abi::load_u16(&t0, abi::stack_pointer(), PFD_SLOT + 6),
        abi::compare_immediate(&t0, "0"),
        abi::branch_eq(&check_stderr),
        abi::move_register(&cur_fd, &fd_out),
        abi::add_immediate(&cur_slot, &file, PROC_STDOUT_BUF),
        abi::move_immediate(&cur_is_err, "Integer", "0"),
        abi::branch(&append),
        abi::label(&check_stderr),
        abi::load_u16(&t0, abi::stack_pointer(), PFD_SLOT + 14),
        abi::compare_immediate(&t0, "0"),
        abi::branch_eq(&wait_loop),
        abi::move_register(&cur_fd, &fd_err),
        abi::add_immediate(&cur_slot, &file, PROC_STDERR_BUF),
        abi::move_immediate(&cur_is_err, "Integer", "1"),
        // --- append the ready stream's bytes to its spill buffer -------------
        abi::label(&append),
        abi::load_u64(&buf, &cur_slot, 0),
        abi::compare_immediate(&buf, "0"),
        abi::branch_ne(&append_have),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &(SPILL_INITIAL_CAPACITY + SPILL_DATA).to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&buf, abi::mfb_return(1)),
        abi::move_immediate(&t0, "Integer", &SPILL_INITIAL_CAPACITY.to_string()),
        abi::store_u64(&t0, &buf, SPILL_CAPACITY),
        abi::store_u64(abi::ZERO, &buf, SPILL_LENGTH),
        abi::store_u64(abi::ZERO, &buf, SPILL_OFFSET),
        abi::store_u64(&buf, &cur_slot, 0),
        abi::label(&append_have),
        abi::load_u64(&cap, &buf, SPILL_CAPACITY),
        abi::load_u64(&len, &buf, SPILL_LENGTH),
        abi::add_immediate(&want, &len, SPILL_CHUNK),
        abi::move_register(&newcap, &cap),
        abi::compare_registers(&want, &newcap),
        abi::branch_le(&cap_ready),
        // Double until a whole chunk fits, then clamp to the stated cap.
        abi::label(&grow_loop),
        abi::shift_left_immediate(&newcap, &newcap, 1),
        abi::compare_registers(&newcap, &want),
        abi::branch_lt(&grow_loop),
        abi::move_immediate(&t0, "Integer", &SPILL_MAX_CAPACITY.to_string()),
        abi::compare_registers(&newcap, &t0),
        abi::branch_le(&do_grow),
        abi::move_register(&newcap, &t0),
        abi::compare_registers(&newcap, &cap),
        abi::branch_le(&cap_ready),
        abi::label(&do_grow),
        abi::add_immediate(abi::return_register(), &newcap, SPILL_DATA),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&t2, abi::mfb_return(1)),
        abi::store_u64(&newcap, &t2, SPILL_CAPACITY),
        abi::store_u64(&len, &t2, SPILL_LENGTH),
        abi::load_u64(&t0, &buf, SPILL_OFFSET),
        abi::store_u64(&t0, &t2, SPILL_OFFSET),
        // Copy the filled prefix a word at a time. `len` may not be a multiple of
        // 8, but both capacities are, so the final partial word is in bounds on
        // each side and the slop past `len` is dead either way.
        abi::add_immediate(&t0, &buf, SPILL_DATA),
        abi::add_immediate(&t1, &t2, SPILL_DATA),
        abi::move_immediate(&size, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&size, &len),
        abi::branch_ge(&copy_done),
        abi::load_u64(&want, &t0, 0),
        abi::store_u64(&want, &t1, 0),
        abi::add_immediate(&t0, &t0, 8),
        abi::add_immediate(&t1, &t1, 8),
        abi::add_immediate(&size, &size, 8),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::move_register(abi::c_arg(0), &buf),
        abi::add_immediate(abi::c_arg(1), &cap, SPILL_DATA),
    ]);
    emit_arena_free(symbol, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(&buf, &t2),
        abi::store_u64(&buf, &cur_slot, 0),
        abi::move_register(&cap, &newcap),
        abi::label(&cap_ready),
        // size = min(SPILL_CHUNK, capacity - length). Zero means the cap is
        // reached and this child cannot be waited for without draining it.
        abi::subtract_registers(&size, &cap, &len),
        abi::move_immediate(&t0, "Integer", &SPILL_CHUNK.to_string()),
        abi::compare_registers(&size, &t0),
        abi::branch_le(&size_ok),
        abi::move_register(&size, &t0),
        abi::label(&size_ok),
        abi::compare_immediate(&size, "0"),
        abi::branch_le(&over_limit),
        abi::label(&read_now),
        abi::move_register(abi::c_arg(0), &cur_fd),
        abi::add_immediate(&t0, &buf, SPILL_DATA),
        abi::add_registers(abi::c_arg(1), &t0, &len),
        abi::move_register(abi::c_arg(2), &size),
    ]);
    platform.emit_external_call(
        "read",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&read_err),
        abi::branch_eq(&mark_eof),
        abi::load_u64(&t0, &buf, SPILL_LENGTH),
        abi::add_registers(&t0, &t0, abi::c_return(0)),
        abi::store_u64(&t0, &buf, SPILL_LENGTH),
        abi::branch(&wait_loop),
        abi::label(&read_err),
    ]);
    platform.emit_errno(
        symbol,
        (&errno).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, EINTR),
        abi::branch_eq(&read_now),
        // Any other read error means this stream can give us nothing more; treat
        // it as end of stream so the wait still makes progress.
        abi::label(&mark_eof),
        abi::compare_immediate(&cur_is_err, "0"),
        abi::branch_ne(&mark_eof_err),
        abi::move_immediate(&eof_out, "Integer", "1"),
        abi::branch(&wait_loop),
        abi::label(&mark_eof_err),
        abi::move_immediate(&eof_err, "Integer", "1"),
        abi::branch(&wait_loop),
        // --- reaped ----------------------------------------------------------
        abi::label(&have_status),
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
        abi::label(&over_limit),
    ]);
    emit_fail(
        symbol,
        "ErrResourceBusy",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed_l));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, PFD_SLOT + 16))
}

/// The Windows `waitFor` body — the same drain-while-you-wait contract as the Unix
/// one (bug-475), over `WaitForSingleObject`/`PeekNamedPipe`/`ReadFile`.
///
/// A Windows anonymous pipe buffers even less than a Unix one, so the old
/// `WaitForSingleObject(hProcess, INFINITE)` deadlocked against a chatty child
/// exactly as `waitpid` did. There is no `poll` over several pipes here, so the
/// loop peeks each still-open pipe in turn and sleeps 1 ms when neither has
/// anything — the shape `process::poll` already uses on this platform.
pub(crate) fn lower_process_waitfor_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    // Explicit Win64 frame (depth-1, no vregs). [0x00..0x20) is the shadow a callee
    // writes into its caller's frame (`call_external` does not reserve it), and
    // [0x20..0x30) carries outgoing stack arguments 5 and 6 — `PeekNamedPipe` takes
    // six. Every value live across a call therefore starts at 0x30.
    const EXIT: usize = 0x30; // GetExitCodeProcess out-param
    const FILE: usize = 0x38; // the Process record pointer
    const AVAIL: usize = 0x40; // PeekNamedPipe out-param
    const NREAD: usize = 0x48; // ReadFile out-param
    const EOF_OUT: usize = 0x50;
    const EOF_ERR: usize = 0x58;
    const CUR_H: usize = 0x60; // the pipe being serviced this pass
    const CUR_SLOT: usize = 0x68; // address of its spill-pointer slot in the record
    const CUR_IS_ERR: usize = 0x70;
    const BUF: usize = 0x78; // its spill block
    const CAP: usize = 0x80;
    const LEN: usize = 0x88;
    const NEWCAP: usize = 0x90;
    const SIZE: usize = 0x98;
    const SRC: usize = 0xA0;
    const DST: usize = 0xA8;
    const IDX: usize = 0xB0;
    const NEWBUF: usize = 0xB8;
    const FRAME: usize = 0xC0;
    // WAIT_TIMEOUT: the one return that means "still running". WAIT_FAILED and
    // WAIT_ABANDONED fall through to the exit path rather than spinning.
    const WAIT_TIMEOUT: &str = "258";
    const INFINITE: &str = "4294967295";

    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let cached = format!("{symbol}_cached");
    let done = format!("{symbol}_done");
    let out_open = format!("{symbol}_out_open");
    let err_open = format!("{symbol}_err_open");
    let wait_loop = format!("{symbol}_wait_loop");
    let have_status = format!("{symbol}_have_status");
    let try_stderr = format!("{symbol}_try_stderr");
    let both_eof = format!("{symbol}_both_eof");
    let peek = format!("{symbol}_peek");
    let append_have = format!("{symbol}_append_have");
    let grow_loop = format!("{symbol}_grow_loop");
    let do_grow = format!("{symbol}_do_grow");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let cap_ready = format!("{symbol}_cap_ready");
    let size_ok = format!("{symbol}_size_ok");
    let stream_eof = format!("{symbol}_stream_eof");
    let stream_eof_err = format!("{symbol}_stream_eof_err");
    let idle = format!("{symbol}_idle");
    let over_limit = format!("{symbol}_over_limit");
    let alloc_fail = format!("{symbol}_alloc_fail");

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
        abi::branch_ne(&cached),
        // Seed the per-stream EOF flags; a handle that is not a live pipe (0 or
        // INVALID_HANDLE_VALUE) is finished before we start.
        abi::store_u64(abi::ZERO, sp, EOF_OUT),
        abi::store_u64(abi::ZERO, sp, EOF_ERR),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_gt(&out_open),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), sp, EOF_OUT),
        abi::label(&out_open),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_gt(&err_open),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), sp, EOF_ERR),
        abi::label(&err_open),
        // --- drain-while-you-wait -------------------------------------------
        abi::label(&wait_loop),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
    ];
    platform.emit_external_call(
        "WaitForSingleObject",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), WAIT_TIMEOUT),
        abi::branch_ne(&have_status),
        // Still running: service one still-open pipe, stdout first.
        abi::load_u64(abi::mfb_arg(0), sp, EOF_OUT),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_ne(&try_stderr),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
        abi::store_u64(abi::mfb_arg(1), sp, CUR_H),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_BUF),
        abi::store_u64(abi::mfb_arg(1), sp, CUR_SLOT),
        abi::store_u64(abi::ZERO, sp, CUR_IS_ERR),
        abi::branch(&peek),
        abi::label(&try_stderr),
        abi::load_u64(abi::mfb_arg(0), sp, EOF_ERR),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_ne(&both_eof),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
        abi::store_u64(abi::mfb_arg(1), sp, CUR_H),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_BUF),
        abi::store_u64(abi::mfb_arg(1), sp, CUR_SLOT),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), sp, CUR_IS_ERR),
        abi::branch(&peek),
        // Both streams finished: nothing of ours can hold the child now.
        abi::label(&both_eof),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", INFINITE),
    ]);
    platform.emit_external_call(
        "WaitForSingleObject",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&have_status),
        // PeekNamedPipe(h, NULL, 0, NULL, &avail, NULL)
        abi::label(&peek),
        abi::store_u64(abi::ZERO, sp, AVAIL),
        abi::add_immediate(abi::mfb_arg(0), sp, AVAIL),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20),
        abi::store_u64(abi::ZERO, sp, 0x28),
        abi::load_u64(abi::mfb_arg(0), sp, CUR_H),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "PeekNamedPipe",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&stream_eof), // FALSE = broken pipe
        abi::load_u32(abi::mfb_arg(0), sp, AVAIL),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&idle),
        // --- append the ready stream's bytes to its spill buffer -------------
        abi::load_u64(abi::mfb_arg(0), sp, CUR_SLOT),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0),
        abi::store_u64(abi::mfb_arg(1), sp, BUF),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&append_have),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &(SPILL_INITIAL_CAPACITY + SPILL_DATA).to_string(),
        ),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(abi::mfb_arg(0), abi::mfb_return(1)),
        abi::store_u64(abi::mfb_arg(0), sp, BUF),
        abi::move_immediate(
            abi::mfb_arg(1),
            "Integer",
            &SPILL_INITIAL_CAPACITY.to_string(),
        ),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), SPILL_CAPACITY),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), SPILL_LENGTH),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), SPILL_OFFSET),
        abi::load_u64(abi::mfb_arg(1), sp, CUR_SLOT),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0),
        abi::label(&append_have),
        abi::load_u64(abi::mfb_arg(0), sp, BUF),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), SPILL_CAPACITY),
        abi::store_u64(abi::mfb_arg(1), sp, CAP),
        abi::store_u64(abi::mfb_arg(1), sp, NEWCAP),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), SPILL_LENGTH),
        abi::store_u64(abi::mfb_arg(2), sp, LEN),
        // want = length + CHUNK
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(2), SPILL_CHUNK),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(1)),
        abi::branch_le(&cap_ready),
        // Double until a whole chunk fits, then clamp to the stated cap.
        abi::label(&grow_loop),
        abi::load_u64(abi::mfb_arg(1), sp, NEWCAP),
        abi::shift_left_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, NEWCAP),
        abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(3)),
        abi::branch_lt(&grow_loop),
        abi::move_immediate(abi::mfb_arg(0), "Integer", &SPILL_MAX_CAPACITY.to_string()),
        abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(0)),
        abi::branch_le(&do_grow),
        abi::store_u64(abi::mfb_arg(0), sp, NEWCAP),
        abi::load_u64(abi::mfb_arg(1), sp, CAP),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_le(&cap_ready),
        abi::label(&do_grow),
        abi::load_u64(abi::mfb_arg(0), sp, NEWCAP),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(0), SPILL_DATA),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(abi::mfb_arg(0), abi::mfb_return(1)),
        abi::store_u64(abi::mfb_arg(0), sp, NEWBUF),
        abi::load_u64(abi::mfb_arg(1), sp, NEWCAP),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), SPILL_CAPACITY),
        abi::load_u64(abi::mfb_arg(1), sp, LEN),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), SPILL_LENGTH),
        abi::load_u64(abi::mfb_arg(2), sp, BUF),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), SPILL_OFFSET),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), SPILL_OFFSET),
        // Copy the filled prefix a word at a time; both capacities are multiples
        // of 8, so the final partial word is in bounds on each side.
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(2), SPILL_DATA),
        abi::store_u64(abi::mfb_arg(1), sp, SRC),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), SPILL_DATA),
        abi::store_u64(abi::mfb_arg(1), sp, DST),
        abi::store_u64(abi::ZERO, sp, IDX),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, IDX),
        abi::load_u64(abi::mfb_arg(1), sp, LEN),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_ge(&copy_done),
        abi::load_u64(abi::mfb_arg(2), sp, SRC),
        abi::load_u64(abi::mfb_arg(3), sp, DST),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), 0),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(3), 0),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 8),
        abi::store_u64(abi::mfb_arg(2), sp, SRC),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 8),
        abi::store_u64(abi::mfb_arg(3), sp, DST),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8),
        abi::store_u64(abi::mfb_arg(0), sp, IDX),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::load_u64(abi::return_register(), sp, BUF),
        abi::load_u64(abi::mfb_arg(1), sp, CAP),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), SPILL_DATA),
    ]);
    emit_arena_free(symbol, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, NEWBUF),
        abi::store_u64(abi::mfb_arg(0), sp, BUF),
        abi::load_u64(abi::mfb_arg(1), sp, CUR_SLOT),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0),
        abi::load_u64(abi::mfb_arg(0), sp, NEWCAP),
        abi::store_u64(abi::mfb_arg(0), sp, CAP),
        // size = min(SPILL_CHUNK, capacity - length); zero means the cap is hit.
        abi::label(&cap_ready),
        abi::load_u64(abi::mfb_arg(0), sp, CAP),
        abi::load_u64(abi::mfb_arg(1), sp, LEN),
        abi::subtract_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &SPILL_CHUNK.to_string()),
        abi::compare_registers(abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::branch_le(&size_ok),
        abi::move_register(abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::label(&size_ok),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_le(&over_limit),
        abi::store_u64(abi::mfb_arg(2), sp, SIZE),
        // ReadFile(h, buf + DATA + length, size, &nread, NULL)
        abi::store_u64(abi::ZERO, sp, 0x20),
        abi::store_u64(abi::ZERO, sp, NREAD),
        abi::load_u64(abi::mfb_arg(1), sp, BUF),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), SPILL_DATA),
        abi::load_u64(abi::mfb_arg(2), sp, LEN),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(2)),
        abi::load_u64(abi::mfb_arg(2), sp, SIZE),
        abi::add_immediate(abi::mfb_arg(3), sp, NREAD),
        abi::load_u64(abi::mfb_arg(0), sp, CUR_H),
    ]);
    platform.emit_external_call(
        "ReadFile",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&stream_eof),
        abi::load_u32(abi::mfb_arg(0), sp, NREAD),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&stream_eof),
        abi::load_u64(abi::mfb_arg(1), sp, BUF),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(1), SPILL_LENGTH),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(0)),
        abi::store_u64(abi::mfb_arg(2), abi::mfb_arg(1), SPILL_LENGTH),
        abi::branch(&wait_loop),
        abi::label(&stream_eof),
        abi::load_u64(abi::mfb_arg(0), sp, CUR_IS_ERR),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_ne(&stream_eof_err),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(0), sp, EOF_OUT),
        abi::branch(&wait_loop),
        abi::label(&stream_eof_err),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(0), sp, EOF_ERR),
        abi::branch(&wait_loop),
        // Nothing to move right now: yield before re-checking the child.
        abi::label(&idle),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "Sleep",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&wait_loop),
        // --- reaped ----------------------------------------------------------
        // GetExitCodeProcess(hProcess, &exit)
        abi::label(&have_status),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::mfb_arg(1), sp, EXIT),
    ]);
    platform.emit_external_call(
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
        abi::label(&over_limit),
    ]);
    emit_fail(
        symbol,
        "ErrResourceBusy",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed_l));
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
