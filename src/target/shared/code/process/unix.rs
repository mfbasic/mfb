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
    platform.emit_libc_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
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
    platform.emit_libc_call(
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
    platform.emit_libc_call(
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
    platform.emit_libc_call(
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
        platform.emit_libc_call(
            "close",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
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
// Frame for a spawn/shell helper: three stdio pipes + a self-pipe (each an
// `int[2]`) + a 4-byte errno readback buffer.
const STDIN_P: usize = 0;
const STDOUT_P: usize = 8;
const STDERR_P: usize = 16;
const ERR_P: usize = 24;
const ERRBUF: usize = 32;
const SPAWN_LOCAL: usize = 48;

/// Copy `len` bytes from `src` into a freshly arena-allocated NUL-terminated C
/// string (allocated `len + 1`), returning it in `mfb_return(1)`. Leaves the
/// result also in the `out` vreg. Runs in the fork child, so the allocation is
/// never freed (the child execs or _exits). `cnt`/`byte`/`sp`/`dp` are scratch
/// vregs. `src`/`len` must survive `emit_alloc` (vregs spill).
#[allow(clippy::too_many_arguments)]
fn emit_copy_to_cstring(
    symbol: &str,
    src: &str,
    len: &str,
    out: &str,
    sp: &str,
    dp: &str,
    cnt: &str,
    byte: &str,
    loop_label: &str,
    done_label: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::add_immediate(abi::return_register(), len, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(out, abi::mfb_return(1)),
        abi::move_register(sp, src),
        abi::move_register(dp, out),
        abi::move_immediate(cnt, "Integer", "0"),
        abi::label(loop_label),
        abi::compare_registers(cnt, len),
        abi::branch_eq(done_label),
        abi::load_u8(byte, sp, 0),
        abi::store_u8(byte, dp, 0),
        abi::add_immediate(sp, sp, 1),
        abi::add_immediate(dp, dp, 1),
        abi::add_immediate(cnt, cnt, 1),
        abi::branch(loop_label),
        abi::label(done_label),
        abi::store_u8(abi::ZERO, dp, 0),
    ]);
}

/// Apply an environment `Map OF String TO String` in the fork child before
/// `execvp`. When `envreplace` is nonzero, first clear the inherited environment
/// (portably: `unsetenv` each current `environ` entry's name — `unsetenv` exists
/// on macOS and Linux, unlike `clearenv`). Then `setenv(name, value, 1)` for each
/// USED map entry. All C strings are arena-allocated and never freed (the child
/// execs immediately).
#[allow(clippy::too_many_arguments)]
fn emit_child_apply_env(
    symbol: &str,
    v: &mut Vregs,
    map: &str,
    envreplace: &str,
    alloc_fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let map0 = v.next();
    let ep = v.next();
    let estr = v.next();
    let nlen = v.next();
    let sp = v.next();
    let dp = v.next();
    let cnt = v.next();
    let byte = v.next();
    let namebuf = v.next();
    let cap = v.next();
    let i = v.next();
    let entry = v.next();
    let dbase = v.next();
    let off = v.next();
    let klen = v.next();
    let vlen = v.next();
    let keyc = v.next();
    let valc = v.next();
    let flags = v.next();

    // Preserve the map pointer across the environ/unsetenv/setenv libc calls.
    instructions.push(abi::move_register(&map0, map));

    // --- optional clear ---
    let no_clear = format!("{symbol}_env_noclear");
    let clear_loop = format!("{symbol}_env_clear");
    let clear_done = format!("{symbol}_env_clear_done");
    let scan_loop = format!("{symbol}_env_scan");
    let scan_done = format!("{symbol}_env_scan_done");
    let name_copy = format!("{symbol}_env_name_copy");
    let name_copy_done = format!("{symbol}_env_name_copy_done");
    instructions.extend([
        abi::compare_immediate(envreplace, "0"),
        abi::branch_eq(&no_clear),
    ]);
    platform.emit_environ_pointer(symbol, platform_imports, instructions, relocations)?;
    instructions.push(abi::move_register(&ep, abi::return_register()));
    instructions.extend([
        abi::label(&clear_loop),
        abi::load_u64(&estr, &ep, 0),
        abi::compare_immediate(&estr, "0"),
        abi::branch_eq(&clear_done),
        // nlen = index of '=' (or NUL) in estr
        abi::move_register(&sp, &estr),
        abi::move_immediate(&nlen, "Integer", "0"),
        abi::label(&scan_loop),
        abi::load_u8(&byte, &sp, 0),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_eq(&scan_done),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&scan_done),
        abi::add_immediate(&sp, &sp, 1),
        abi::add_immediate(&nlen, &nlen, 1),
        abi::branch(&scan_loop),
        abi::label(&scan_done),
    ]);
    emit_copy_to_cstring(
        symbol,
        &estr,
        &nlen,
        &namebuf,
        &sp,
        &dp,
        &cnt,
        &byte,
        &name_copy,
        &name_copy_done,
        alloc_fail,
        instructions,
        relocations,
    );
    instructions.push(abi::move_register(abi::c_arg(0), &namebuf));
    platform.emit_libc_call(
        "unsetenv",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    // environ shifted down in place; environ[0] is the next entry. Reload ep in
    // case the accessor is not stable, then loop.
    platform.emit_environ_pointer(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::move_register(&ep, abi::return_register()),
        abi::branch(&clear_loop),
        abi::label(&clear_done),
        abi::label(&no_clear),
    ]);

    // --- setenv each USED map entry ---
    let ent_loop = format!("{symbol}_env_ent");
    let ent_done = format!("{symbol}_env_ent_done");
    let ent_skip = format!("{symbol}_env_ent_skip");
    let key_copy = format!("{symbol}_env_key_copy");
    let key_copy_done = format!("{symbol}_env_key_copy_done");
    let val_copy = format!("{symbol}_env_val_copy");
    let val_copy_done = format!("{symbol}_env_val_copy_done");
    instructions.extend([
        abi::load_u64(&cap, &map0, LIST_CAP),
        // dbase = map + HEADER + cap*ENTRY
        abi::move_immediate(&off, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&dbase, &cap, &off),
        abi::add_immediate(&dbase, &dbase, LIST_HEADER),
        abi::add_registers(&dbase, &map0, &dbase),
        abi::move_immediate(&i, "Integer", "0"),
        abi::label(&ent_loop),
        abi::compare_registers(&i, &cap),
        abi::branch_eq(&ent_done),
        // entry = map + HEADER + i*ENTRY
        abi::move_immediate(&off, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&entry, &i, &off),
        abi::add_immediate(&entry, &entry, LIST_HEADER),
        abi::add_registers(&entry, &map0, &entry),
        // skip unused slots (tombstones)
        abi::load_u8(&flags, &entry, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::move_immediate(&byte, "Integer", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::and_registers(&flags, &flags, &byte),
        abi::compare_immediate(&flags, "0"),
        abi::branch_eq(&ent_skip),
        // key cstr
        abi::load_u64(&off, &entry, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers(&sp, &dbase, &off),
        abi::load_u64(&klen, &entry, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
    ]);
    emit_copy_to_cstring(
        symbol,
        &sp,
        &klen,
        &keyc,
        &dp,
        &namebuf,
        &cnt,
        &byte,
        &key_copy,
        &key_copy_done,
        alloc_fail,
        instructions,
        relocations,
    );
    instructions.extend([
        // value cstr — recompute entry (vregs survive, but reload offsets fresh)
        abi::move_immediate(&off, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&entry, &i, &off),
        abi::add_immediate(&entry, &entry, LIST_HEADER),
        abi::add_registers(&entry, &map0, &entry),
        abi::load_u64(&off, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers(&sp, &dbase, &off),
        abi::load_u64(&vlen, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
    ]);
    emit_copy_to_cstring(
        symbol,
        &sp,
        &vlen,
        &valc,
        &dp,
        &namebuf,
        &cnt,
        &byte,
        &val_copy,
        &val_copy_done,
        alloc_fail,
        instructions,
        relocations,
    );
    // setenv(key, value, 1)
    instructions.extend([
        abi::move_register(abi::c_arg(0), &keyc),
        abi::move_register(abi::c_arg(1), &valc),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "setenv",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::label(&ent_skip),
        abi::add_immediate(&i, &i, 1),
        abi::branch(&ent_loop),
        abi::label(&ent_done),
    ]);
    Ok(())
}

/// Shared spawn tail: given a fully-built NUL-terminated C `argv` (in the `argv`
/// vreg), create the three stdio pipes + an O_CLOEXEC self-pipe, fork, and
/// `execvp` in the child (reporting an exec failure to the parent over the
/// self-pipe), then in the parent allocate + stamp the `Process` record. Branches
/// to `fork_fail` on a pipe/fork failure, `alloc_fail` on OOM, `done` on success.
/// Emits its own `child`/`spawn_fail` labels; the caller emits the
/// `fork_fail`/`alloc_fail`/`done` labels. Draws its scratch vregs from the
/// caller's `Vregs` so nothing collides.
#[allow(clippy::too_many_arguments)]
fn emit_spawn_tail(
    symbol: &str,
    v: &mut Vregs,
    argv: &str,
    // Optional child-side setup applied AFTER the dup2/close dance and BEFORE
    // `execvp`: a working-directory C-string ptr (skipped when its first byte is
    // NUL, i.e. an empty cwd) and an environment `(map vreg, envReplace vreg)`.
    cwd: Option<&str>,
    env: Option<(&str, &str)>,
    alloc_fail: &str,
    fork_fail: &str,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    const F_SETFD: &str = "2";
    const FD_CLOEXEC: &str = "1";
    let pid = v.next();
    let rec = v.next();
    let tmp = v.next();
    let errno = v.next();
    let child = format!("{symbol}_child");
    let spawn_fail = format!("{symbol}_spawn_fail");
    // Create the three stdio pipes and the self-pipe.
    for off in [STDIN_P, STDOUT_P, STDERR_P, ERR_P] {
        instructions.push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), off));
        platform.emit_libc_call("pipe", symbol, platform_imports, instructions, relocations)?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_lt(fork_fail),
        ]);
    }
    // Self-pipe write end O_CLOEXEC: closed automatically on a successful exec, so
    // the parent's read returns EOF; left open on exec failure to carry errno.
    instructions.extend([
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P + 4),
        abi::move_immediate(abi::c_arg(1), "Integer", F_SETFD),
        abi::move_immediate(abi::c_arg(2), "Integer", FD_CLOEXEC),
    ]);
    platform.emit_variadic_call("fcntl", symbol, platform_imports, instructions, relocations)?;
    // fork()
    platform.emit_libc_call("fork", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(&pid, abi::c_return(0)),
        abi::compare_immediate(&pid, "0"),
        abi::branch_eq(&child),
        abi::branch_lt(fork_fail),
    ]);
    // ---- parent ----
    for slot in [STDIN_P, STDOUT_P + 4, STDERR_P + 4, ERR_P + 4] {
        instructions.push(abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot));
        platform.emit_libc_call("close", symbol, platform_imports, instructions, relocations)?;
    }
    instructions.extend([
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ERRBUF),
        abi::move_immediate(abi::c_arg(2), "Integer", "4"),
    ]);
    platform.emit_libc_call("read", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_gt(&spawn_fail),
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P),
    ]);
    platform.emit_libc_call("close", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
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
        abi::branch(done),
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
        platform.emit_libc_call("dup2", symbol, platform_imports, instructions, relocations)?;
    }
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
        platform.emit_libc_call("close", symbol, platform_imports, instructions, relocations)?;
    }
    // Child-side working directory + environment, applied before execvp (safe in
    // the single-threaded fork child). A chdir/env failure is best-effort: the
    // subsequent execvp still runs and any real failure surfaces via the
    // self-pipe as ErrSpawnFailed.
    if let Some(cwd) = cwd {
        let skip_chdir = format!("{symbol}_skip_chdir");
        let byte = v.next();
        instructions.extend([
            abi::load_u8(&byte, cwd, 0),
            abi::compare_immediate(&byte, "0"),
            abi::branch_eq(&skip_chdir),
            abi::move_register(abi::c_arg(0), cwd),
        ]);
        platform.emit_libc_call("chdir", symbol, platform_imports, instructions, relocations)?;
        instructions.push(abi::label(&skip_chdir));
    }
    if let Some((map, envreplace)) = env {
        emit_child_apply_env(
            symbol,
            v,
            map,
            envreplace,
            alloc_fail,
            platform,
            platform_imports,
            instructions,
            relocations,
        )?;
    }
    instructions.extend([
        abi::load_u64(abi::c_arg(0), argv, 0),
        abi::move_register(abi::c_arg(1), argv),
    ]);
    platform.emit_libc_call(
        "execvp",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    platform.emit_errno(
        symbol,
        errno.as_str().into(),
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::store_u32(&errno, abi::stack_pointer(), ERRBUF),
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P + 4),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ERRBUF),
        abi::move_immediate(abi::c_arg(2), "Integer", "4"),
    ]);
    platform.emit_libc_call("write", symbol, platform_imports, instructions, relocations)?;
    instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", "127"));
    platform.emit_libc_call("_exit", symbol, platform_imports, instructions, relocations)?;
    // ---- exec-failure reap (no zombie) ----
    instructions.push(abi::label(&spawn_fail));
    instructions.extend([
        abi::move_register(abi::c_arg(0), &pid),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "waitpid",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::branch(fork_fail));
    Ok(())
}

/// Emit `store_u8` bytes materializing a NUL-terminated ASCII literal at
/// `dst + 0..`, using `byte` as a scratch vreg. The buffer must already be
/// allocated with room for `text.len() + 1` bytes.
fn emit_cstring_literal(
    text: &str,
    dst: &str,
    byte: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    for (offset, ch) in text.bytes().enumerate() {
        instructions.push(abi::move_immediate(byte, "Integer", &ch.to_string()));
        instructions.push(abi::store_u8(byte, dst, offset));
    }
    instructions.push(abi::store_u8(abi::ZERO, dst, text.len()));
}

pub(in crate::target::shared::code) fn lower_process_spawn_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    _with_env: bool,
) -> HelperResult {
    const LOCAL: usize = SPAWN_LOCAL;

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

    let invalid = format!("{symbol}_invalid_args");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let build_loop = format!("{symbol}_argv_loop");
    let build_done = format!("{symbol}_argv_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let fork_fail = format!("{symbol}_fork_fail");
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
    emit_spawn_tail(
        symbol,
        &mut v,
        &argv,
        None,
        None,
        &alloc_fail,
        &fork_fail,
        &done,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
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

// ---------------------------------------------------------------------------
// process.shell — run a command line through `/bin/sh -c`. Builds the fixed
// argv `["/bin/sh", "-c", cmd]` and reuses the shared spawn tail. (`/bin/sh` on
// both macOS and Linux — the plan's bash-on-macOS preference is dropped for
// portability; see Corrections.)
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_shell_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const LOCAL: usize = SPAWN_LOCAL;
    let mut v = Vregs::new();
    let cmdstr = v.next();
    let argv = v.next();
    let cstr = v.next();
    let srcp = v.next();
    let dstp = v.next();
    let len = v.next();
    let j = v.next();
    let byte = v.next();
    let alloc_fail = format!("{symbol}_alloc_fail");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let fork_fail = format!("{symbol}_fork_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&cmdstr, abi::return_register()),
        // argv = alloc(4*8, 8)  ["/bin/sh", "-c", cmd, NULL]
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ];
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::move_register(&argv, abi::mfb_return(1)));
    // argv[0] = "/bin/sh"
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "8"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::move_register(&cstr, abi::mfb_return(1)));
    emit_cstring_literal("/bin/sh", &cstr, &byte, &mut instructions);
    instructions.push(abi::store_u64(&cstr, &argv, 0));
    // argv[1] = "-c"
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "3"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::move_register(&cstr, abi::mfb_return(1)));
    emit_cstring_literal("-c", &cstr, &byte, &mut instructions);
    instructions.push(abi::store_u64(&cstr, &argv, 8));
    // argv[2] = cmd (copy the String's bytes into a fresh NUL-terminated cstr)
    instructions.extend([
        abi::load_u64(&len, &cmdstr, 0),
        abi::add_immediate(abi::return_register(), &len, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&cstr, abi::mfb_return(1)),
        abi::add_immediate(&srcp, &cmdstr, 8),
        abi::move_register(&dstp, &cstr),
        abi::move_immediate(&j, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&j, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &srcp, 0),
        abi::store_u8(&byte, &dstp, 0),
        abi::add_immediate(&srcp, &srcp, 1),
        abi::add_immediate(&dstp, &dstp, 1),
        abi::add_immediate(&j, &j, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dstp, 0),
        abi::store_u64(&cstr, &argv, 16),
        // argv[3] = NULL
        abi::store_u64(abi::ZERO, &argv, 24),
    ]);
    emit_spawn_tail(
        symbol,
        &mut v,
        &argv,
        None,
        None,
        &alloc_fail,
        &fork_fail,
        &done,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&fork_fail));
    emit_fail(
        symbol,
        ERR_SPAWN_FAILED_CODE,
        ERR_SPAWN_FAILED_SYMBOL,
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

// ---------------------------------------------------------------------------
// process.spawnEnv — the full spawn(args, cwd, env, envReplace) form. Builds the
// C argv from `args` and a cwd C-string from `cwd`, then runs the shared tail
// with child-side chdir + environment application (setenv per entry; the whole
// inherited environment cleared first when envReplace is true).
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_spawnenv_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const LOCAL: usize = SPAWN_LOCAL;
    let mut v = Vregs::new();
    let args = v.next();
    let cwd_str = v.next();
    let env_map = v.next();
    let envrep = v.next();
    let cwdptr = v.next();
    let cwdlen = v.next();
    let cwdcstr = v.next();
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
    let sp = v.next();
    let dp = v.next();
    let cnt = v.next();

    let invalid = format!("{symbol}_invalid_args");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let cwd_copy = format!("{symbol}_cwd_copy");
    let cwd_copy_done = format!("{symbol}_cwd_copy_done");
    let build_loop = format!("{symbol}_argv_loop");
    let build_done = format!("{symbol}_argv_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let fork_fail = format!("{symbol}_fork_fail");
    let done = format!("{symbol}_done");

    // Capture the four arguments (x0..x3) before any clobbering libc call.
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&args, abi::return_register()),
        abi::move_register(&cwd_str, abi::c_arg(1)),
        abi::move_register(&env_map, abi::c_arg(2)),
        abi::move_register(&envrep, abi::c_arg(3)),
        abi::load_u64(&n, &args, LIST_COUNT),
        abi::compare_immediate(&n, "0"),
        abi::branch_eq(&invalid),
    ];
    let mut relocations = Vec::new();
    // cwd C string (empty cwd → "\0", whose leading NUL makes the child skip chdir).
    instructions.extend([
        abi::add_immediate(&cwdptr, &cwd_str, 8),
        abi::load_u64(&cwdlen, &cwd_str, 0),
    ]);
    emit_copy_to_cstring(
        symbol,
        &cwdptr,
        &cwdlen,
        &cwdcstr,
        &sp,
        &dp,
        &cnt,
        &byte,
        &cwd_copy,
        &cwd_copy_done,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    // Build argv from the args list (same entry-array walk as spawn).
    instructions.extend([
        abi::load_u64(&cap, &args, LIST_CAP),
        abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&cap, &cap, &tmp),
        abi::add_immediate(&cap, &cap, LIST_HEADER),
        abi::add_registers(&dbase, &args, &cap),
        abi::add_immediate(&tmp, &n, 1),
        abi::move_immediate(&byte, "Integer", "8"),
        abi::multiply_registers(&tmp, &tmp, &byte),
        abi::move_register(abi::return_register(), &tmp),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&argv, abi::mfb_return(1)),
        abi::move_immediate(&i, "Integer", "0"),
        abi::label(&build_loop),
        abi::compare_registers(&i, &n),
        abi::branch_eq(&build_done),
        abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&entry, &i, &tmp),
        abi::add_immediate(&entry, &entry, LIST_HEADER),
        abi::add_registers(&entry, &args, &entry),
        abi::load_u64(&srcp, &entry, ENTRY_VOFF),
        abi::add_registers(&srcp, &dbase, &srcp),
        abi::load_u64(&vlen, &entry, ENTRY_VLEN),
        abi::add_immediate(abi::return_register(), &vlen, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&cstr, abi::mfb_return(1)),
        abi::move_immediate(&tmp, "Integer", "8"),
        abi::multiply_registers(&tmp, &i, &tmp),
        abi::add_registers(&tmp, &argv, &tmp),
        abi::store_u64(&cstr, &tmp, 0),
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
        abi::move_immediate(&tmp, "Integer", "8"),
        abi::multiply_registers(&tmp, &n, &tmp),
        abi::add_registers(&tmp, &argv, &tmp),
        abi::store_u64(abi::ZERO, &tmp, 0),
    ]);
    emit_spawn_tail(
        symbol,
        &mut v,
        &argv,
        Some(&cwdcstr),
        Some((&env_map, &envrep)),
        &alloc_fail,
        &fork_fail,
        &done,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
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

// ---------------------------------------------------------------------------
// process.send / process.sendBytes — write to the child's stdin (the parent's
// write end). `send` writes the String bytes then a trailing '\n'; `sendBytes`
// writes the raw List OF Byte with no newline. Blocking (partial-write loop with
// EINTR retry); a broken pipe (child stdin gone) raises ErrResourceClosed.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_send_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    is_bytes: bool,
    with_timeout: bool,
) -> HelperResult {
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
        abi::label("entry"),
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
    platform.emit_libc_call(
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
        errno.as_str().into(),
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
        platform.emit_libc_call(
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
            errno.as_str().into(),
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
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    if with_timeout {
        instructions.push(abi::label(&timeout_l));
        emit_fail(
            symbol,
            ERR_TIMEOUT_CODE,
            ERR_TIMEOUT_SYMBOL,
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 32);
    Ok((frame, instructions, relocations, stack_slots))
}

const POLLOUT: &str = "4";

/// Emit a `poll(&{fd, events, 0}, 1, timeout)` on the pollfd staged at
/// `sp + pollfd_slot`; branch to `timeout_l` on a `0` (timed-out) return. `events`
/// is `POLLIN`/`POLLOUT`. `scratch` is a caller vreg for the sign-extended return.
/// A `< 0` poll error (e.g. EINTR) falls through and the following blocking op
/// re-polls — acceptable since a spurious wakeup just retries.
#[allow(clippy::too_many_arguments)]
fn emit_poll_wait(
    symbol: &str,
    fd: &str,
    timeout: &str,
    scratch: &str,
    pollfd_slot: usize,
    events: &str,
    timeout_l: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::store_u32(fd, abi::stack_pointer(), pollfd_slot),
        abi::move_immediate(scratch, "Integer", events),
        abi::store_u16(scratch, abi::stack_pointer(), pollfd_slot + 4),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), pollfd_slot + 6),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), pollfd_slot),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), timeout),
    ]);
    platform.emit_libc_call("poll", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(scratch, abi::c_return(0)),
        abi::compare_immediate(scratch, "0"),
        abi::branch_eq(timeout_l),
    ]);
    Ok(())
}

// ---------------------------------------------------------------------------
// process.poll — is the selected child stream readable within `ms` ms? Returns
// true if readable OR at EOF (POLLHUP makes poll return > 0), so a draining
// receive can follow; false on timeout. `with_from` selects the stream via the
// `Stream` arg (0 = StdOut, else StdErr); the 2-arg form always polls stdout.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_poll_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_from: bool,
) -> HelperResult {
    const POLLFD_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let ms = v.next();
    let fd = v.next();
    let n = v.next();
    let from = v.next();
    let closed_l = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&ms, abi::c_arg(1)),
    ];
    if with_from {
        instructions.push(abi::move_register(&from, abi::c_arg(2)));
    }
    instructions.extend([
        abi::load_u64(&n, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&n, "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::compare_immediate(&from, "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(&fd, &file, PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(&fd, &file, PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(&fd, &file, PROC_STDOUT_R));
    }
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u32(&fd, abi::stack_pointer(), POLLFD_SLOT),
        abi::move_immediate(&n, "Integer", "1"), // POLLIN
        abi::store_u16(&n, abi::stack_pointer(), POLLFD_SLOT + 4),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), POLLFD_SLOT + 6),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), POLLFD_SLOT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), &ms),
    ]);
    platform.emit_libc_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(&n, abi::c_return(0)),
        abi::compare_immediate(&n, "0"),
        abi::branch_gt(&ready),
        // 0 (timeout) or < 0 (error) -> not ready.
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
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
// process.receiveBytes — return the next available chunk of raw bytes from the
// selected stream. Reads one `read()` into a temporary buffer; a pipe read
// returns any buffered bytes before EOF, so late output is drained. On EOF (an
// empty read) with nothing buffered, raises ErrResourceClosed. `with_from`
// selects stderr; the 1-arg form reads stdout.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_receivebytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_from: bool,
) -> HelperResult {
    const FD_OFFSET: usize = 0;
    const N_OFFSET: usize = 8;
    const BUF_OFFSET: usize = 16;
    const CHUNK: &str = "65536";
    const EINTR: &str = "4";
    let closed = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let read_retry = format!("{symbol}_read_retry");
    let read_fail = format!("{symbol}_read_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v9", abi::return_register(), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
    ];
    if with_from {
        instructions.extend([
            abi::compare_immediate(abi::c_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64("%v9", abi::return_register(), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64("%v9", abi::return_register(), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64("%v9", abi::return_register(), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        // Allocate the temporary chunk buffer.
        abi::move_immediate(abi::return_register(), "Integer", CHUNK),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFFSET),
        abi::label(&read_retry),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate(abi::c_arg(2), "Integer", CHUNK),
    ]);
    platform.emit_libc_call(
        "read",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&read_fail),
        abi::branch_eq(&closed), // EOF with nothing buffered
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), N_OFFSET),
    ]);
    // Build a List OF Byte with N elements from BUF (mirrors net.read).
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
        abi::move_immediate("%v11", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::mfb_return(1)),
        abi::move_immediate("%v9", "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::load_u64("%v15", abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&entry_done),
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    instructions.extend([
        abi::add_registers("%v12", "%v14", "%v9"),
        abi::load_u8("%v13", "%v15", 0),
        abi::store_u8("%v13", "%v12", 0),
        abi::add_immediate("%v15", "%v15", 1),
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.push(abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE));
    }
    instructions.extend([
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&read_fail),
    ]);
    platform.emit_errno(
        symbol,
        ("%v9").into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", EINTR),
        abi::branch_eq(&read_retry),
        abi::label(&closed),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
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
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 32);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.receive — return one '\n'-terminated line (including the newline) from
// the selected stream, as a validated String. Reads a byte at a time into a
// line accumulator, so it never over-reads past the line boundary (no cross-call
// buffering is needed). On EOF the accumulated bytes are returned even without a
// trailing newline (drain-before-close); EOF with an empty line raises
// ErrResourceClosed. `with_from` selects stderr; the 1-arg form reads stdout.
// (A byte-at-a-time read trades a syscall per byte for a buffer-free, always-
// correct line framing; the chunk-oriented `receiveBytes` is the bulk path.)
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_receive_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_from: bool,
) -> HelperResult {
    const FD: usize = 0;
    const LINEP: usize = 8; // accumulator pointer (for the result build)
    const N: usize = 16; // accumulated length
    const STR: usize = 24; // built String ptr
    const CAP: usize = 1048576; // 1 MiB max line
    const EINTR: &str = "4";

    let closed = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_fail = format!("{symbol}_read_fail");
    let got_byte = format!("{symbol}_got_byte");
    let eof_check = format!("{symbol}_eof_check");
    let build = format!("{symbol}_build");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v9", abi::return_register(), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
    ];
    if with_from {
        instructions.extend([
            abi::compare_immediate(abi::c_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64("%v9", abi::return_register(), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64("%v9", abi::return_register(), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64("%v9", abi::return_register(), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64("%v9", abi::stack_pointer(), FD),
        // Accumulator buffer.
        abi::move_immediate(abi::return_register(), "Integer", &CAP.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), LINEP),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), N),
        // read one byte into acc[filled].
        abi::label(&read_loop),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), FD),
        abi::load_u64("%v9", abi::stack_pointer(), LINEP),
        abi::load_u64("%v10", abi::stack_pointer(), N),
        abi::add_registers(abi::c_arg(1), "%v9", "%v10"),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "read",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&read_fail),
        abi::branch_gt(&got_byte),
        // r == 0: EOF.
        abi::label(&eof_check),
        abi::load_u64("%v10", abi::stack_pointer(), N),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&closed),
        abi::branch(&build),
        abi::label(&got_byte),
        // filled += 1; check the byte just read for '\n'.
        abi::load_u64("%v9", abi::stack_pointer(), LINEP),
        abi::load_u64("%v10", abi::stack_pointer(), N),
        abi::add_registers("%v11", "%v9", "%v10"),
        abi::load_u8("%v12", "%v11", 0),
        abi::add_immediate("%v10", "%v10", 1),
        abi::store_u64("%v10", abi::stack_pointer(), N),
        abi::compare_immediate("%v12", "10"), // '\n'
        abi::branch_eq(&build),
        abi::move_immediate("%v11", "Integer", &CAP.to_string()),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_eq(&build), // line too long -> return what we have
        abi::branch(&read_loop),
        abi::label(&read_fail),
    ]);
    platform.emit_errno(
        symbol,
        ("%v9").into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", EINTR),
        abi::branch_eq(&read_loop),
        abi::branch(&closed),
        abi::label(&build),
    ]);
    super::super::net::emit_string_result_build(
        symbol,
        LINEP,
        N,
        STR,
        &str_copy,
        &str_done,
        &alloc_fail,
        &encoding_error,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    emit_fail(
        symbol,
        ERR_ENCODING_CODE,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
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
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 48);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.signal — deliver a Signal bucket to the child. Kill->SIGKILL,
// Terminate->SIGTERM, Error->SIGABRT, None->no-op. Operating on a dropped/
// detached process raises ErrResourceClosed.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_signal_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let sig = v.next();
    let num = v.next();
    let closed_l = format!("{symbol}_closed");
    let set_kill = format!("{symbol}_set_kill");
    let set_term = format!("{symbol}_set_term");
    let do_kill = format!("{symbol}_do_kill");
    let done_ok = format!("{symbol}_done_ok");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&sig, abi::c_arg(1)),
        abi::load_u64(&num, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&num, "0"),
        abi::branch_ne(&closed_l),
        // None (0) -> no-op.
        abi::compare_immediate(&sig, "0"),
        abi::branch_eq(&done_ok),
        abi::compare_immediate(&sig, "1"),
        abi::branch_eq(&set_kill),
        abi::compare_immediate(&sig, "2"),
        abi::branch_eq(&set_term),
        // Error (3) -> SIGABRT.
        abi::move_immediate(&num, "Integer", "6"),
        abi::branch(&do_kill),
        abi::label(&set_kill),
        abi::move_immediate(&num, "Integer", "9"),
        abi::branch(&do_kill),
        abi::label(&set_term),
        abi::move_immediate(&num, "Integer", "15"),
        abi::label(&do_kill),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::move_register(abi::c_arg(1), &num),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "kill",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&done_ok),
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
// process.didSignal — the Signal bucket a TERMINATED child died on (read from the
// cached raw waitpid status): None if it exited normally or has not terminated;
// Kill for SIGKILL; Error for the fault signals (SIGILL/ABRT/FPE/BUS/SEGV);
// Terminate for every other terminating signal.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_didsignal_helper(
    symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let reaped = v.next();
    let status = v.next();
    let termsig = v.next();
    let closed_l = format!("{symbol}_closed");
    let ret_none = format!("{symbol}_ret_none");
    let ret_kill = format!("{symbol}_ret_kill");
    let ret_error = format!("{symbol}_ret_error");
    let ret_term = format!("{symbol}_ret_term");
    let ret = format!("{symbol}_ret");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&reaped, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_eq(&ret_none),
        abi::load_u64(&status, &file, PROC_STATUS),
        abi::move_immediate(&termsig, "Integer", "127"),
        abi::and_registers(&termsig, &status, &termsig),
        abi::compare_immediate(&termsig, "0"),
        abi::branch_eq(&ret_none),
        abi::compare_immediate(&termsig, "9"),
        abi::branch_eq(&ret_kill),
        // Error bucket: SIGILL(4)/SIGABRT(6)/SIGFPE(8)/SIGBUS(10)/SIGSEGV(11).
        abi::compare_immediate(&termsig, "4"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "6"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "8"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "10"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "11"),
        abi::branch_eq(&ret_error),
        // Everything else -> Terminate.
        abi::label(&ret_term),
        abi::move_immediate(&termsig, "Integer", "2"),
        abi::branch(&ret),
        abi::label(&ret_none),
        abi::move_immediate(&termsig, "Integer", "0"),
        abi::branch(&ret),
        abi::label(&ret_kill),
        abi::move_immediate(&termsig, "Integer", "1"),
        abi::branch(&ret),
        abi::label(&ret_error),
        abi::move_immediate(&termsig, "Integer", "3"),
        abi::label(&ret),
        abi::move_register(RESULT_VALUE_REGISTER, &termsig),
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
// process.detach — relinquish ownership WITHOUT killing: close the parent-side
// pipe fds, set SIGCHLD to SIG_IGN so the kernel auto-reaps the (now un-waited)
// child and no zombie is left, and set the record `closed` bit so scope-drop's
// __drop is a no-op and any later op traps ErrResourceClosed.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_detach_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let sigchld = if platform.family() == PlatformFamily::MacOS {
        "20"
    } else {
        "17"
    };
    let mut v = Vregs::new();
    let file = v.next();
    let fd = v.next();
    let one = v.next();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
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
        platform.emit_libc_call(
            "close",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::label(&skip));
    }
    // signal(SIGCHLD, SIG_IGN=1) -> kernel auto-reaps, no zombie.
    instructions.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", sigchld),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "signal",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, RESOURCE_OFFSET_CLOSED),
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
