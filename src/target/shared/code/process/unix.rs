//! Unix native backend for the `process` package: fork + exec + three pipes +
//! waitpid/kill, emitted as self-contained runtime helpers. Every libc call goes
//! through `platform.emit_libc_call` so the four Unix targets (macOS-aarch64,
//! Linux x86_64/aarch64/riscv64) share one body. Values live in numeric virtual
//! registers (`Vregs`; the shared allocator spills them across each `bl`); only
//! genuine memory a syscall fills — the `pipe(int[2])` arrays, the `waitpid`
//! status int, the `read` errno buffer, the built `argv` array — uses the
//! explicit `sp`-relative frame from `finalize_vreg_body_with_locals`.

use super::*;
use crate::target::shared::abi;
use std::collections::HashMap;

// POSIX constants identical across macOS and Linux for the ops here.
const SIGKILL: &str = "9";
const WNOHANG: &str = "1";

// Collection layout (a `List OF String`): count/capacity in the header, each
// element's bytes stored inline in the data region and addressed by the entry's
// (valueOffset, valueLength). Data region base = coll + HEADER + capacity*ENTRY.
const LIST_COUNT: usize = COLLECTION_OFFSET_COUNT;
const LIST_CAP: usize = COLLECTION_OFFSET_CAPACITY;
const LIST_HEADER: usize = COLLECTION_HEADER_SIZE;
const LIST_ENTRY: usize = COLLECTION_ENTRY_SIZE;
const ENTRY_VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
const ENTRY_VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;

/// Emit the resource-record `(tag, value)` failure for a code + message symbol,
/// then branch to `done` — the process analogue of `net`'s `emit_fail`.
fn emit_fail(
    symbol: &str,
    code: &str,
    message_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, message_symbol, instructions, relocations);
    instructions.push(abi::branch(done));
}

/// Decode a raw `waitpid` status word (`status` vreg) into an exit code (`exit`
/// vreg): `WIFEXITED` → `(status >> 8) & 0xff`, otherwise signal-death → `-1`.
/// `s0`/`s1` are caller-supplied scratch vregs distinct from `status`/`exit`.
fn emit_decode_status(
    status: &str,
    exit: &str,
    s0: &str,
    s1: &str,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
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
// process.pid — read the cached child pid (handle@8).
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_pid_helper(
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
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.close — close the child's stdin (parent's write end), once.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let fd = v.next();
    let neg = v.next();
    let closed_l = format!("{symbol}_closed");
    let already = format!("{symbol}_stdin_already");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&fd, &file, PROC_STDIN_W),
        abi::compare_immediate(&fd, "0"),
        abi::branch_lt(&already),
        abi::move_register(abi::c_arg(0), &fd),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::bitwise_not(&neg, abi::ZERO), // -1: stdin marked closed
        abi::store_u64(&neg, &file, PROC_STDIN_W),
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.waitFor — block until exit, return the exit code (-1 on signal).
// Idempotent: a second call returns the cached code.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_waitfor_helper(
    symbol: &str,
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
    platform.emit_libc_call("waitpid", symbol, platform_imports, &mut instructions, &mut relocations)?;
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
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.isRunning — WNOHANG waitpid; caches the exit state on a reap.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_isrunning_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
        abi::label("entry"),
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
    platform.emit_libc_call("waitpid", symbol, platform_imports, &mut instructions, &mut relocations)?;
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
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.__drop — scope-drop: SIGKILL + reap a live child, close pipe fds, set
// the closed bit. Idempotent (a closed record is a no-op).
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_drop_helper(
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
    platform.emit_libc_call("kill", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STATUS_SLOT),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_libc_call("waitpid", symbol, platform_imports, &mut instructions, &mut relocations)?;
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
        platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.push(abi::label(&skip));
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
// process.spawn — argv-only. Builds a NUL-terminated C `argv` from a
// `List OF String`, creates three stdio pipes + a close-on-exec self-pipe,
// forks, and execvp's in the child. The child reports an exec failure to the
// parent over the self-pipe (the parent's read returns >0 bytes = errno);
// a successful exec closes the O_CLOEXEC self-pipe, so the parent reads EOF (0).
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_spawn_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    _with_env: bool,
) -> HelperResult {
    // Frame: three stdio pipes + a self-pipe (each an `int[2]`) + a 4-byte errno
    // readback buffer.
    const STDIN_P: usize = 0;
    const STDOUT_P: usize = 8;
    const STDERR_P: usize = 16;
    const ERR_P: usize = 24;
    const ERRBUF: usize = 32;
    const LOCAL: usize = 48;
    const F_SETFD: &str = "2";
    const FD_CLOEXEC: &str = "1";

    let mut v = Vregs::new();
    let list = v.next();
    let n = v.next();
    let cap = v.next();
    let dbase = v.next();
    let argv = v.next();
    let i = v.next();
    let entry = v.next();
    let vlen = v.next();
    let srcp = v.next();
    let dstp = v.next();
    let cstr = v.next();
    let j = v.next();
    let byte = v.next();
    let tmp = v.next();
    let pid = v.next();
    let rec = v.next();
    let errno = v.next();

    let invalid = format!("{symbol}_invalid_args");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let build_loop = format!("{symbol}_argv_loop");
    let build_done = format!("{symbol}_argv_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let child = format!("{symbol}_child");
    let fork_fail = format!("{symbol}_fork_fail");
    let spawn_fail = format!("{symbol}_spawn_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&list, abi::return_register()),
        abi::load_u64(&n, &list, LIST_COUNT),
        abi::compare_immediate(&n, "0"),
        abi::branch_eq(&invalid),
        // dbase = list + HEADER + cap*ENTRY
        abi::load_u64(&cap, &list, LIST_CAP),
        abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&cap, &cap, &tmp),
        abi::add_immediate(&cap, &cap, LIST_HEADER),
        abi::add_registers(&dbase, &list, &cap),
        // argv = alloc((n+1)*8, 8)
        abi::add_immediate(&tmp, &n, 1),
        abi::move_immediate(&byte, "Integer", "8"),
        abi::multiply_registers(&tmp, &tmp, &byte),
        abi::move_register(abi::return_register(), &tmp),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ];
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&argv, abi::mfb_return(1)),
        abi::move_immediate(&i, "Integer", "0"),
        abi::label(&build_loop),
        abi::compare_registers(&i, &n),
        abi::branch_eq(&build_done),
        // entry = list + HEADER + i*ENTRY
        abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&entry, &i, &tmp),
        abi::add_immediate(&entry, &entry, LIST_HEADER),
        abi::add_registers(&entry, &list, &entry),
        abi::load_u64(&srcp, &entry, ENTRY_VOFF),
        abi::add_registers(&srcp, &dbase, &srcp),
        abi::load_u64(&vlen, &entry, ENTRY_VLEN),
        // cstr = alloc(vlen+1, 1)
        abi::add_immediate(abi::return_register(), &vlen, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&cstr, abi::mfb_return(1)),
        // argv[i] = cstr
        abi::move_immediate(&tmp, "Integer", "8"),
        abi::multiply_registers(&tmp, &i, &tmp),
        abi::add_registers(&tmp, &argv, &tmp),
        abi::store_u64(&cstr, &tmp, 0),
        // copy vlen bytes srcp -> cstr, NUL-terminate
        abi::move_register(&dstp, &cstr),
        abi::move_immediate(&j, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&j, &vlen),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &srcp, 0),
        abi::store_u8(&byte, &dstp, 0),
        abi::add_immediate(&srcp, &srcp, 1),
        abi::add_immediate(&dstp, &dstp, 1),
        abi::add_immediate(&j, &j, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dstp, 0),
        abi::add_immediate(&i, &i, 1),
        abi::branch(&build_loop),
        abi::label(&build_done),
        // argv[n] = NULL
        abi::move_immediate(&tmp, "Integer", "8"),
        abi::multiply_registers(&tmp, &n, &tmp),
        abi::add_registers(&tmp, &argv, &tmp),
        abi::store_u64(abi::ZERO, &tmp, 0),
    ]);
    // Create the three stdio pipes and the self-pipe.
    for off in [STDIN_P, STDOUT_P, STDERR_P, ERR_P] {
        instructions.push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), off));
        platform.emit_libc_call("pipe", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_lt(&fork_fail),
        ]);
    }
    // Self-pipe write end O_CLOEXEC: closed automatically on a successful exec, so
    // the parent's read returns EOF; left open on exec failure to carry errno.
    instructions.extend([
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P + 4),
        abi::move_immediate(abi::c_arg(1), "Integer", F_SETFD),
        abi::move_immediate(abi::c_arg(2), "Integer", FD_CLOEXEC),
    ]);
    platform.emit_variadic_call("fcntl", symbol, platform_imports, &mut instructions, &mut relocations)?;
    // fork()
    platform.emit_libc_call("fork", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::sign_extend_word(&pid, abi::c_return(0)),
        abi::compare_immediate(&pid, "0"),
        abi::branch_eq(&child),
        abi::branch_lt(&fork_fail),
    ]);
    // ---- parent ----
    // Close the child ends: stdin read, stdout write, stderr write, self write.
    for slot in [STDIN_P, STDOUT_P + 4, STDERR_P + 4, ERR_P + 4] {
        instructions.push(abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot));
        platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
    }
    // read(self read end, &errno, 4): >0 => child failed exec.
    instructions.extend([
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ERRBUF),
        abi::move_immediate(abi::c_arg(2), "Integer", "4"),
    ]);
    platform.emit_libc_call("read", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_gt(&spawn_fail),
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P),
    ]);
    platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
    // Allocate + stamp the Process record.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&rec, abi::mfb_return(1)),
        abi::move_immediate(&tmp, "Integer", RESOURCE_TAG_PROCESS),
        abi::store_u64(&tmp, &rec, RESOURCE_OFFSET_TAG),
        abi::store_u64(&pid, &rec, RESOURCE_OFFSET_HANDLE),
        abi::store_u64(abi::ZERO, &rec, RESOURCE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, &rec, RESOURCE_OFFSET_STATE),
        abi::load_u32(&tmp, abi::stack_pointer(), STDIN_P + 4),
        abi::store_u64(&tmp, &rec, PROC_STDIN_W),
        abi::load_u32(&tmp, abi::stack_pointer(), STDOUT_P),
        abi::store_u64(&tmp, &rec, PROC_STDOUT_R),
        abi::load_u32(&tmp, abi::stack_pointer(), STDERR_P),
        abi::store_u64(&tmp, &rec, PROC_STDERR_R),
        abi::store_u64(abi::ZERO, &rec, PROC_REAPED),
        abi::store_u64(abi::ZERO, &rec, PROC_STATUS),
        abi::store_u64(abi::ZERO, &rec, PROC_EXITCODE),
        abi::store_u64(abi::ZERO, &rec, 80),
        abi::store_u64(abi::ZERO, &rec, 88),
        abi::move_register(RESULT_VALUE_REGISTER, &rec),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // ---- child ----
    instructions.push(abi::label(&child));
    for (off, hi, target) in [
        (STDIN_P, false, "0"),
        (STDOUT_P, true, "1"),
        (STDERR_P, true, "2"),
    ] {
        let slot = if hi { off + 4 } else { off };
        instructions.extend([
            abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot),
            abi::move_immediate(abi::c_arg(1), "Integer", target),
        ]);
        platform.emit_libc_call("dup2", symbol, platform_imports, &mut instructions, &mut relocations)?;
    }
    // Close every pipe fd in the child EXCEPT the O_CLOEXEC self-pipe write end
    // (ERR_P + 4), which the kernel closes on a successful exec.
    for slot in [
        STDIN_P,
        STDIN_P + 4,
        STDOUT_P,
        STDOUT_P + 4,
        STDERR_P,
        STDERR_P + 4,
        ERR_P,
    ] {
        instructions.push(abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot));
        platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
    }
    // execvp(argv[0], argv)
    instructions.extend([
        abi::load_u64(abi::c_arg(0), &argv, 0),
        abi::move_register(abi::c_arg(1), &argv),
    ]);
    platform.emit_libc_call("execvp", symbol, platform_imports, &mut instructions, &mut relocations)?;
    // exec failed: write errno to the self-pipe, _exit(127).
    platform.emit_errno(symbol, errno.as_str().into(), platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::store_u32(&errno, abi::stack_pointer(), ERRBUF),
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P + 4),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ERRBUF),
        abi::move_immediate(abi::c_arg(2), "Integer", "4"),
    ]);
    platform.emit_libc_call("write", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", "127"));
    platform.emit_libc_call("_exit", symbol, platform_imports, &mut instructions, &mut relocations)?;
    // ---- error exits ----
    instructions.push(abi::label(&spawn_fail));
    // Reap the failed child so no zombie: waitpid(pid, NULL, 0).
    instructions.extend([
        abi::move_register(abi::c_arg(0), &pid),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_libc_call("waitpid", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::branch(&fork_fail));
    instructions.push(abi::label(&fork_fail));
    emit_fail(
        symbol,
        ERR_SPAWN_FAILED_CODE,
        ERR_SPAWN_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCAL);
    Ok((frame, instructions, relocations, stack_slots))
}
