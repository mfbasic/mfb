//! Unix native backend for the `process` package: fork + exec + three pipes +
//! waitpid/kill, emitted as self-contained runtime helpers. The exec is `execvp`
//! on Linux and `posix_spawnp` with `POSIX_SPAWN_SETEXEC` on macOS — the latter
//! is an exec that also carries `POSIX_SPAWN_CLOEXEC_DEFAULT` and
//! `POSIX_SPAWN_SETSIGDEF` (bug-543). Every libc call goes
//! through `platform.emit_external_call` so the four Unix targets (macOS-aarch64,
//! Linux x86_64/aarch64/riscv64) share one body. Values live in numeric virtual
//! registers (`Vregs`; the shared allocator spills them across each `bl`); only
//! genuine memory a syscall fills — the `pipe(int[2])` arrays, the `waitpid`
//! status int, the `read` errno buffer, the built `argv` array — uses the
//! explicit `sp`-relative frame from `finalize_vreg_body_with_locals`.

// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::os::syscall::{SIGPIPE_SIGNO, SIG_DFL};
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod spawn;
pub(crate) use spawn::*;

// POSIX constants identical across macOS and Linux for the ops here.
pub(crate) const SIGKILL: &str = "9";
pub(crate) const WNOHANG: &str = "1";

// Collection layout (a `List OF String`): count/capacity in the header, each
// element's bytes stored inline in the data region and addressed by the entry's
// (valueOffset, valueLength). Data region base = coll + HEADER + capacity*ENTRY.
pub(crate) const LIST_COUNT: usize = COLLECTION_OFFSET_COUNT;
pub(crate) const LIST_CAP: usize = COLLECTION_OFFSET_CAPACITY;
pub(crate) const LIST_HEADER: usize = COLLECTION_HEADER_SIZE;
pub(crate) const LIST_ENTRY: usize = COLLECTION_ENTRY_SIZE;
pub(crate) const ENTRY_VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
pub(crate) const ENTRY_VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;

/// Decode a raw `waitpid` status word (`status` vreg) into an exit code (`exit`
/// vreg): `WIFEXITED` → `(status >> 8) & 0xff`, otherwise signal-death → `-1`.
/// `s0`/`s1` are caller-supplied scratch vregs distinct from `status`/`exit`.
pub(crate) fn emit_decode_status(
    status: impl Into<Reg>,
    exit: impl Into<Reg>,
    s0: impl Into<Reg>,
    s1: impl Into<Reg>,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let exit: Reg = exit.into();
    let (status, s0, s1): (Reg, Reg, Reg) = (status.into(), s0.into(), s1.into());
    let signaled = format!("{symbol}_signaled");
    let decoded = format!("{symbol}_decoded");
    instructions.extend([
        // termsig = status & 0x7f
        abi::move_immediate(s0, "Integer", "127"),
        abi::and_registers(s0, status, s0),
        abi::compare_immediate(s0, "0"),
        abi::branch_ne(&signaled),
        // WIFEXITED: exit = (status >> 8) & 0xff
        abi::shift_right_immediate(exit, status, 8),
        abi::move_immediate(s1, "Integer", "255"),
        abi::and_registers(exit, exit, s1),
        abi::branch(&decoded),
        abi::label(&signaled),
        abi::bitwise_not(exit, abi::ZERO), // WIFSIGNALED → -1
        abi::label(&decoded),
    ]);
}
// ---------------------------------------------------------------------------
// process.__drop — scope-drop: SIGKILL + reap a live child, close pipe fds, set
// the closed bit. Idempotent (a closed record is a no-op).
// ---------------------------------------------------------------------------
pub(crate) fn lower_process_drop_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const STATUS_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let reaped = v.next();
    let fd = v.next();
    let one = v.next();
    let done = format!("{symbol}_done");
    let done_ok = format!("{symbol}_done_ok");
    let already_reaped = format!("{symbol}_already_reaped");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&done_ok),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&already_reaped),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::c_arg(1), "Integer", SIGKILL),
    ];
    let mut relocations = Vec::new();
    platform.emit_external_call(
        "kill",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
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
    instructions.push(abi::label(&already_reaped));
    // Close the three retained pipe fds if still open (>= 0).
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
    // Hand back the two spill blocks `waitFor` may have grown (bug-475). Each can
    // reach SPILL_MAX_CAPACITY, so leaving them parked on the arena for the rest of
    // the program would be a real retention on a program that supervises several
    // chatty children. The handle is closed on this path, so nothing can read them
    // afterwards.
    for off in [PROC_STDOUT_BUF, PROC_STDERR_BUF] {
        let skip = format!("{symbol}_skip_spill_{off}");
        instructions.extend([
            abi::load_u64(&fd, &file, off),
            abi::compare_immediate(&fd, "0"),
            abi::branch_eq(&skip),
            abi::load_u64(&one, &fd, SPILL_CAPACITY),
            abi::add_immediate(abi::c_arg(1), &one, SPILL_DATA),
            abi::move_register(abi::return_register(), &fd),
        ]);
        emit_arena_free(symbol, &mut instructions, &mut relocations);
        instructions.extend([abi::store_u64(abi::ZERO, &file, off), abi::label(&skip)]);
    }
    instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, RESOURCE_OFFSET_CLOSED),
        abi::label(&done_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.send / process.sendBytes — write to the child's stdin (the parent's
// write end). `send` writes the String bytes then a trailing '\n'; `sendBytes`
// writes the raw List OF Byte with no newline. Blocking (partial-write loop with
// EINTR retry); a broken pipe (child stdin gone) raises ErrResourceClosed.
// ---------------------------------------------------------------------------
pub(crate) fn lower_process_send_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    is_bytes: bool,
    with_timeout: bool,
) -> Result<ProcBodyParts, String> {
    const NL_SLOT: usize = 0;
    const POLLFD_SLOT: usize = 8; // { int fd; short events; short revents }
    const EINTR: &str = "4";
    let mut v = Vregs::new();
    let file = v.next();
    let val = v.next();
    let fd = v.next();
    let srcp = v.next();
    let rem = v.next();
    let n = v.next();
    let errno = v.next();
    let timeout = v.next();
    let s0 = v.next();
    let s1 = v.next();
    let s2 = v.next();
    let closed_l = format!("{symbol}_closed");
    let write_loop = format!("{symbol}_write_loop");
    let write_fail = format!("{symbol}_write_fail");
    let payload_done = format!("{symbol}_payload_done");
    let nl_loop = format!("{symbol}_nl_loop");
    let nl_fail = format!("{symbol}_nl_fail");
    let nl_done = format!("{symbol}_nl_done");
    let timeout_l = format!("{symbol}_timeout");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&val, abi::c_arg(1)),
    ];
    if with_timeout {
        instructions.push(abi::move_register(&timeout, abi::c_arg(2)));
    }
    instructions.extend([
        abi::load_u64(&n, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&n, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&fd, &file, PROC_STDIN_W),
        abi::compare_immediate(&fd, "0"),
        abi::branch_lt(&closed_l),
    ]);
    let mut relocations = Vec::new();
    if is_bytes {
        instructions.push(abi::load_u64(&rem, &val, COLLECTION_OFFSET_COUNT));
        push_collection_data_base_from_capacity(&mut instructions, &srcp, &val, &s0, &s1, &s2);
    } else {
        instructions.extend([
            abi::load_u64(&rem, &val, 0),
            abi::add_immediate(&srcp, &val, 8),
        ]);
    }
    instructions.extend([
        abi::label(&write_loop),
        abi::compare_immediate(&rem, "0"),
        abi::branch_eq(&payload_done),
    ]);
    if with_timeout {
        emit_poll_wait(
            symbol,
            &fd,
            &timeout,
            &n,
            POLLFD_SLOT,
            POLLOUT,
            &timeout_l,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::move_register(abi::c_arg(0), &fd),
        abi::move_register(abi::c_arg(1), &srcp),
        abi::move_register(abi::c_arg(2), &rem),
    ]);
    platform.emit_external_call(
        "write",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(&n, abi::c_return(0)),
        abi::compare_immediate(&n, "0"),
        abi::branch_le(&write_fail),
        abi::add_registers(&srcp, &srcp, &n),
        abi::subtract_registers(&rem, &rem, &n),
        abi::branch(&write_loop),
        abi::label(&write_fail),
    ]);
    platform.emit_errno(
        symbol,
        errno.into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, EINTR),
        abi::branch_eq(&write_loop),
        abi::branch(&closed_l),
        abi::label(&payload_done),
    ]);
    if !is_bytes {
        instructions.extend([
            abi::move_immediate(&n, "Integer", "10"),
            abi::store_u8(&n, abi::stack_pointer(), NL_SLOT),
            abi::label(&nl_loop),
        ]);
        if with_timeout {
            emit_poll_wait(
                symbol,
                &fd,
                &timeout,
                &n,
                POLLFD_SLOT,
                POLLOUT,
                &timeout_l,
                platform,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.extend([
            abi::move_register(abi::c_arg(0), &fd),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), NL_SLOT),
            abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        ]);
        platform.emit_external_call(
            "write",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(&n, abi::c_return(0)),
            abi::compare_immediate(&n, "0"),
            abi::branch_gt(&nl_done),
            abi::label(&nl_fail),
        ]);
        platform.emit_errno(
            symbol,
            errno.into(),
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&errno, EINTR),
            abi::branch_eq(&nl_loop),
            abi::branch(&closed_l),
            abi::label(&nl_done),
        ]);
    }
    instructions.extend([
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
    if with_timeout {
        instructions.push(abi::label(&timeout_l));
        emit_fail(
            symbol,
            "ErrTimeout",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 32))
}

pub(crate) const POLLOUT: &str = "4";

/// Emit a `poll(&{fd, events, 0}, 1, timeout)` on the pollfd staged at
/// `sp + pollfd_slot`; branch to `timeout_l` on a `0` (timed-out) return. `events`
/// is `POLLIN`/`POLLOUT`. `scratch` is a caller vreg for the sign-extended return.
/// A `< 0` poll error (e.g. EINTR) falls through and the following blocking op
/// re-polls — acceptable since a spurious wakeup just retries.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_poll_wait(
    symbol: &str,
    fd: impl Into<Reg>,
    timeout: impl Into<Reg>,
    scratch: impl Into<Reg>,
    pollfd_slot: usize,
    events: &str,
    timeout_l: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let (fd, timeout, scratch): (Reg, Reg, Reg) = (fd.into(), timeout.into(), scratch.into());
    instructions.extend([
        abi::store_u32(fd, abi::stack_pointer(), pollfd_slot),
        abi::move_immediate(scratch, "Integer", events),
        abi::store_u16(scratch, abi::stack_pointer(), pollfd_slot + 4),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), pollfd_slot + 6),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), pollfd_slot),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), timeout),
    ]);
    platform.emit_external_call("poll", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(scratch, abi::c_return(0)),
        abi::compare_immediate(scratch, "0"),
        abi::branch_eq(timeout_l),
    ]);
    Ok(())
}

// ---------------------------------------------------------------------------
// `_mfb_rt_process_reaper` (bug-474) — the pthread start routine `process::detach`
// spawns to reap exactly ONE detached child.
// ---------------------------------------------------------------------------

/// `void *_mfb_rt_process_reaper(void *pid)` — `waitpid(pid, NULL, 0); return NULL;`.
///
/// `detach` used to arrange the child's cleanup by flipping the **process-wide**
/// `SIGCHLD` disposition to `SIG_IGN`, which tells the kernel to auto-reap *every*
/// child of the program. Any later `waitpid` then failed with `ECHILD`, and
/// `process::waitFor` reads `ECHILD` as "already reaped" and returns the handle's
/// cached exit code — `0` for a child nobody had waited on. One `detach` therefore
/// silently zeroed the exit status of every other child (bug-474). Reaping on a
/// per-pid thread keeps the disposition untouched, so `waitFor` on a handle that was
/// never detached still reports the child's real status.
///
/// The child pid arrives **by value** in the C first-argument register, never a
/// pointer to the `Process` record: the record's arena block may be reclaimed at the
/// detaching scope's exit while this thread is still blocked in `waitpid`.
///
/// **Callee-saved registers.** pthread IS the caller of a start routine and keeps its
/// own live state in the callee-saved bank, so clobbering one aborts at *thread exit*
/// (`_pthread_terminate` PAC failure / libmalloc in `_pthread_tsd_cleanup`) with none
/// of this code in the backtrace. The body DOES use one: `pid` is live across
/// `waitpid`, so the allocator colors it callee-saved (`x21` on macos/linux-aarch64,
/// `r12` on linux-x86_64) — and `finalize_vreg_helper`'s frame builder saves and
/// restores exactly the registers the allocator used (`calleeSaved: ["x21","lr"]` in
/// the dump), which is what makes that safe. The one register it must NOT touch is
/// `ARENA_STATE_REGISTER` (`x19`), and that is reserved from allocation, so it cannot.
/// This is the difference from `lower_thread_trampoline`, which hand-manages its frame
/// (and therefore has to save `x19`/`x20`/the closure register itself).
///
/// **Stack alignment.** A start routine is entered by a foreign `call` on x86-64
/// (glibc `start_thread`, musl's dispatch), i.e. at `sp % 16 == 8`, and a downstream
/// call from a frame that assumed `sp % 16 == 0` faults on libc's first `movaps` to a
/// stack local. This body needs no manual bias because `finalize_frame` already adds
/// the per-arch one — `frame_call_padding()` is 8 on x86-64 and 0 on AArch64/RISC-V
/// (`vreg_frame.rs`), which is why the emitted `stackSize` is 24 on linux-x86_64 and
/// 16 elsewhere. The trampoline hand-rolls the same `+8` only because it bypasses this
/// frame builder.
///
/// **Arena.** Arena state is per-thread and a spawned thread gets its own zeroed copy,
/// so a reaper must not allocate, free, or read through `x19`. It does not: the whole
/// body is register moves, `waitpid`, the errno accessor, and a return.
///
/// **Stack size.** `detach` passes a NULL `pthread_attr_t` rather than the explicit
/// 8 MiB `thread::start` and the graphics thread set, because the reason those need it
/// does not apply here: they run arbitrary MFB code, whose frames are routinely
/// hundreds of KiB. This thread runs no MFB code at all. Its own frame is 16 bytes (24
/// on linux-x86_64, the `stackSize` in the dump) plus libc's `waitpid`/errno frames,
/// against a 512 KiB macOS / 128 KiB musl default — four orders of magnitude of
/// headroom, and fixed, since the body cannot grow without editing this function.
/// Taking the default also keeps the per-detach cost small, which matters because the
/// thread count scales with the number of *live* detached children (`func_detach.rs`).
pub(crate) fn lower_process_reaper_helper(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const EINTR: &str = "4";
    let symbol = PROCESS_REAPER_SYMBOL;
    let mut v = Vregs::new();
    let pid = v.next();
    let ret = v.next();
    let errno = v.next();
    let wait_loop = format!("{symbol}_wait");
    let interrupted = format!("{symbol}_interrupted");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&pid, abi::c_arg(0)),
        // waitpid(pid, NULL, 0) — block until this one child exits, then reap it.
        // The status is discarded: nothing can observe a detached child's exit
        // (its handle is closed), so there is nowhere to cache it.
        abi::label(&wait_loop),
        abi::move_register(abi::c_arg(0), &pid),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ];
    let mut relocations = Vec::new();
    platform.emit_external_call(
        "waitpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Retry on EINTR. Signal dispositions and delivery are process-wide, so ANY
    // signal the program takes while this thread sits in `waitpid` returns
    // `-1`/`EINTR` here — and a `waitpid` that returns without reaping leaves
    // exactly the zombie this thread exists to prevent, intermittently. Any other
    // failure (`ECHILD`: something else already reaped the child) is terminal, so
    // the loop exits rather than spinning.
    instructions.extend([
        abi::sign_extend_word(&ret, abi::c_return(0)),
        abi::compare_immediate(&ret, "0"),
        abi::branch_lt(&interrupted),
        abi::branch(&done),
        abi::label(&interrupted),
    ]);
    platform.emit_errno(
        symbol,
        errno.into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, EINTR),
        abi::branch_eq(&wait_loop),
        abi::label(&done),
        abi::move_immediate(abi::c_return(0), "Integer", "0"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "process.reaper",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}
