//! Windows (`win_x86_64`) native backend for the `process` package (plan-90-D).
//!
//! Reimplements the `process` surface over Win32 — `CreateProcessA` +
//! `WaitForSingleObject`/`GetExitCodeProcess` + `TerminateProcess` — sharing the
//! tag-10 record and 96-byte envelope. The handle word (`RESOURCE_OFFSET_HANDLE`)
//! holds the process `HANDLE`; the child pid is cached in `PROC_STATUS`,
//! the exit code in `PROC_EXITCODE`. Landed in phases, gated by the `win_x86_64`
//! capability list (a call whose capability is not advertised never reaches its
//! helper, so the `unimplemented_on_windows` arms below are unreachable
//! placeholders, not live stubs).

use super::*;
use std::collections::HashMap;

/// Standard `Result` error tail (code/message) then branch to `done`.
fn win_fail(
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
    crate::target::shared::code::data_objects::push_error_message_address(
        symbol,
        message_symbol,
        instructions,
        relocations,
    );
    instructions.push(abi::branch(done));
}

fn unimplemented_on_windows(op: &str) -> HelperResult {
    Err(format!(
        "process::{op} native Windows backend is not yet emitted (plan-90-D)"
    ))
}

pub(in crate::target::shared::code) fn lower_process_spawnenv_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("spawn")
}

pub(in crate::target::shared::code) fn lower_process_shell_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("shell")
}

// process.isRunning — GetExitCodeProcess; running iff the code is STILL_ACTIVE
// (259). Otherwise the child has exited: cache the raw code (PROC_STATUS for
// `didSignal`, PROC_EXITCODE for `waitFor`) and return false. The documented
// STILL_ACTIVE ambiguity (a child that genuinely exits with 259 reads as running)
// is accepted, matching the plan.
pub(in crate::target::shared::code) fn lower_process_isrunning_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
        abi::label("entry"),
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
    platform.emit_libc_call(
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
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

// process.close — close the parent's stdin write handle (signals the child's
// stdin EOF); mark it -1. Idempotent per-record via the -1 sentinel.
pub(in crate::target::shared::code) fn lower_process_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let already = format!("{symbol}_already");
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
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), PROC_STDIN_W),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_lt(&already), // -1 sentinel: already closed
    ];
    platform.emit_libc_call(
        "CloseHandle",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, FILE),
        // -1 sentinel (no negative immediate on Win64): 0 - 1.
        abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), PROC_STDIN_W),
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Emit a blocking `WriteFile(fd, [src_slot], [rem_slot], &[written_slot], NULL)`
/// loop that drains `rem` bytes from `src` on the pipe handle in `fd_slot`. The
/// `lpOverlapped` (5th, stack) arg goes at `sp+0x20`. On a `FALSE` return or a
/// zero-byte write (broken pipe) it branches to `fail`. `tag` disambiguates the
/// per-call labels (payload vs the trailing newline).
#[allow(clippy::too_many_arguments)]
fn emit_write_all(
    symbol: &str,
    tag: &str,
    sp: &str,
    fd_slot: usize,
    src_slot: usize,
    rem_slot: usize,
    written_slot: usize,
    fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let loop_l = format!("{symbol}_{tag}_loop");
    let progress = format!("{symbol}_{tag}_progress");
    let done = format!("{symbol}_{tag}_done");
    instructions.extend([
        abi::label(&loop_l),
        abi::load_u64(abi::mfb_arg(1), sp, rem_slot),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_eq(&done),
        // WriteFile(fd, src, rem, &written, NULL)
        abi::load_u64(abi::mfb_arg(0), sp, fd_slot),
        abi::load_u64(abi::mfb_arg(1), sp, src_slot),
        abi::load_u64(abi::mfb_arg(2), sp, rem_slot),
        abi::add_immediate(abi::mfb_arg(3), sp, written_slot),
        abi::store_u64(abi::ZERO, sp, 0x20),
    ]);
    platform.emit_libc_call("WriteFile", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(fail),
        abi::load_u32(abi::mfb_arg(0), sp, written_slot),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_le(fail),
        abi::label(&progress),
        abi::load_u64(abi::mfb_arg(1), sp, src_slot),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(0)),
        abi::store_u64(abi::mfb_arg(1), sp, src_slot),
        abi::load_u64(abi::mfb_arg(1), sp, rem_slot),
        abi::subtract_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(0)),
        abi::store_u64(abi::mfb_arg(1), sp, rem_slot),
        abi::branch(&loop_l),
        abi::label(&done),
    ]);
    Ok(())
}

// process.send / sendBytes / sendTimeout / sendBytesTimeout — write the payload
// to the child's stdin (parent's write end), then a trailing '\n' for the String
// form. `is_bytes` writes the raw List OF Byte contiguously (no newline);
// `with_timeout` accepts the extra `timeoutMs` arg. Windows anonymous pipes have
// no clean write-readiness poll, so the timeout is a best-effort upper bound: the
// blocking WriteFile returns immediately for a draining reader (the common case)
// and does not preempt a genuinely full pipe (a documented Windows limit,
// mirroring `didSignal`).
pub(in crate::target::shared::code) fn lower_process_send_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    is_bytes: bool,
    _with_timeout: bool,
) -> HelperResult {
    const WRITTEN: usize = 0x28;
    const FILE: usize = 0x30;
    const FD: usize = 0x38;
    const SRCP: usize = 0x40;
    const REM: usize = 0x48;
    const LF: usize = 0x50;
    const FRAME: usize = 0x60;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    // The payload pointer arrives in the 2nd MFB arg; stash it before it is
    // clobbered, then derive (srcp, rem).
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::store_u64(abi::mfb_arg(1), sp, SRCP), // payload object (temp)
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDIN_W),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_lt(&closed_l), // stdin -1 sentinel / closed
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        abi::load_u64(abi::mfb_arg(0), sp, SRCP), // payload object
    ];
    if is_bytes {
        // List OF Byte: count in the header, bytes contiguous at data + HEADER
        // (byte_list_entry_stride() == 0).
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_COUNT),
            abi::store_u64(abi::mfb_arg(1), sp, REM),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), COLLECTION_HEADER_SIZE),
            abi::store_u64(abi::mfb_arg(0), sp, SRCP),
        ]);
    } else {
        // String: length@0, bytes@8.
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0),
            abi::store_u64(abi::mfb_arg(1), sp, REM),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8),
            abi::store_u64(abi::mfb_arg(0), sp, SRCP),
        ]);
    }
    emit_write_all(
        symbol, "payload", sp, FD, SRCP, REM, WRITTEN, &closed_l, platform, platform_imports,
        &mut instructions, &mut relocations,
    )?;
    if !is_bytes {
        // Trailing newline (line framing, matching the Unix send).
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(0), "Integer", "10"),
            abi::store_u8(abi::mfb_arg(0), sp, LF),
            abi::add_immediate(abi::mfb_arg(0), sp, LF),
            abi::store_u64(abi::mfb_arg(0), sp, SRCP),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(0), sp, REM),
        ]);
        emit_write_all(
            symbol, "nl", sp, FD, SRCP, REM, WRITTEN, &closed_l, platform, platform_imports,
            &mut instructions, &mut relocations,
        )?;
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

// process.receive / receiveFrom — one '\n'-terminated line (including the newline)
// from the selected stream, as a validated String. Reads a byte at a time so it
// never over-reads past the line boundary. EOF returns the accumulated bytes even
// without a trailing newline; EOF on an empty line raises ErrResourceClosed.
pub(in crate::target::shared::code) fn lower_process_receive_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_from: bool,
) -> HelperResult {
    const NREAD: usize = 0x28;
    const FILE: usize = 0x30;
    const FD: usize = 0x38;
    const ACC: usize = 0x40;
    const N: usize = 0x48;
    const STR: usize = 0x50;
    const I: usize = 0x58;
    const FROM: usize = 0x60;
    const FRAME: usize = 0x70;
    const CAP: usize = 1048576; // 1 MiB max line
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let read_loop = format!("{symbol}_read_loop");
    let eof = format!("{symbol}_eof");
    let build = format!("{symbol}_build");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
    ];
    if with_from {
        instructions.push(abi::store_u64(abi::mfb_arg(1), sp, FROM));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), sp, FROM),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        // accumulator = arena_alloc(CAP, 1)
        abi::move_immediate(abi::return_register(), "Integer", &CAP.to_string()),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, ACC),
        abi::store_u64(abi::ZERO, sp, N),
        abi::label(&read_loop),
        // ReadFile(fd, acc + n, 1, &nread, NULL)
        abi::load_u64(abi::mfb_arg(0), sp, FD),
        abi::load_u64(abi::mfb_arg(1), sp, ACC),
        abi::load_u64(abi::mfb_arg(2), sp, N),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(2)),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "1"),
        abi::add_immediate(abi::mfb_arg(3), sp, NREAD),
        abi::store_u64(abi::ZERO, sp, 0x20),
    ]);
    platform.emit_libc_call("ReadFile", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&eof), // ReadFile FALSE = broken pipe / EOF
        abi::load_u32(abi::mfb_arg(0), sp, NREAD),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&eof),
        // got a byte: n += 1; check for '\n' or a full line.
        abi::load_u64(abi::mfb_arg(0), sp, ACC),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::load_u8(abi::mfb_arg(3), abi::mfb_arg(2), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, N),
        abi::compare_immediate(abi::mfb_arg(3), "10"),
        abi::branch_eq(&build),
        abi::move_immediate(abi::mfb_arg(2), "Integer", &CAP.to_string()),
        abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(2)),
        abi::branch_eq(&build),
        abi::branch(&read_loop),
        abi::label(&eof),
        abi::load_u64(abi::mfb_arg(0), sp, N),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&closed_l),
        abi::label(&build),
        // String obj = arena_alloc(n + 9, 8): length@0, bytes@8, NUL.
        abi::load_u64(abi::mfb_arg(0), sp, N),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(0), 9),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, STR),
        abi::move_register(abi::mfb_arg(0), abi::mfb_return(1)),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0),
        // copy n bytes: dst = str + 8 (mfb_arg(2)), src = acc (mfb_arg(3))
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(0), 8),
        abi::load_u64(abi::mfb_arg(3), sp, ACC),
        abi::store_u64(abi::ZERO, sp, I),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(3), 0),
        abi::store_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, I),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, abi::mfb_arg(2), 0), // NUL-terminate
        // validate_utf8(str + 8, n)
        abi::load_u64(abi::mfb_arg(0), sp, STR),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(0), 8),
        abi::load_u64(abi::c_arg(1), sp, N),
    ]);
    super::super::codegen_utils::emit_call_validate_utf8(
        symbol,
        &encoding_error,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, sp, STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    win_fail(
        symbol,
        ERR_ENCODING_CODE,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed_l));
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    win_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

// process.receiveBytes / receiveBytesFrom — one `ReadFile` chunk from the selected
// stream, returned as a List OF Byte. A broken pipe / zero-byte read is EOF with
// nothing buffered → ErrResourceClosed. `with_from` selects stderr.
pub(in crate::target::shared::code) fn lower_process_receivebytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_from: bool,
) -> HelperResult {
    const NREAD: usize = 0x28;
    const FILE: usize = 0x30;
    const FD: usize = 0x38;
    const BUF: usize = 0x40;
    const BLOCK: usize = 0x48;
    const N: usize = 0x50;
    const I: usize = 0x58;
    const FROM: usize = 0x60;
    const FRAME: usize = 0x70;
    const CHUNK: &str = "65536";
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
    ];
    if with_from {
        instructions.push(abi::store_u64(abi::mfb_arg(1), sp, FROM));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), sp, FROM),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        // chunk buffer = arena_alloc(CHUNK, 1)
        abi::move_immediate(abi::return_register(), "Integer", CHUNK),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, BUF),
        // ReadFile(fd, buf, CHUNK, &nread, NULL)
        abi::load_u64(abi::mfb_arg(0), sp, FD),
        abi::load_u64(abi::mfb_arg(1), sp, BUF),
        abi::move_immediate(abi::mfb_arg(2), "Integer", CHUNK),
        abi::add_immediate(abi::mfb_arg(3), sp, NREAD),
        abi::store_u64(abi::ZERO, sp, 0x20),
    ]);
    platform.emit_libc_call("ReadFile", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&closed_l), // ReadFile FALSE = broken pipe / EOF
        abi::load_u32(abi::mfb_arg(0), sp, NREAD),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&closed_l), // 0 bytes = EOF, nothing buffered
        abi::store_u64(abi::mfb_arg(0), sp, N),
        // result block = arena_alloc(HEADER + n, 8)  (byte-list stride 0)
        abi::add_immediate(abi::return_register(), abi::mfb_arg(0), COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, BLOCK),
        abi::move_register(abi::mfb_arg(0), abi::mfb_return(1)),
        abi::move_immediate(abi::mfb_arg(1), "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_KIND),
        abi::move_immediate(abi::mfb_arg(1), "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(abi::mfb_arg(1), "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(abi::mfb_arg(1), "Byte", "1"),
        abi::store_u8(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_COUNT),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_DATA_CAPACITY),
        // copy n bytes: dst = block + HEADER (mfb_arg(2)), src = buf (mfb_arg(3))
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(0), COLLECTION_HEADER_SIZE),
        abi::load_u64(abi::mfb_arg(3), sp, BUF),
        abi::store_u64(abi::ZERO, sp, I),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(3), 0),
        abi::store_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, I),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::load_u64(RESULT_VALUE_REGISTER, sp, BLOCK),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    win_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

// process.poll / pollFrom — is the selected child stream readable within `ms`?
// Anonymous pipes have no waitable readiness object, so this polls `PeekNamedPipe`
// on a `GetTickCount64` deadline, sleeping 1ms between probes. Returns true when
// bytes are buffered OR the pipe is broken (EOF — so a draining receive can
// follow); false on timeout. `with_from` selects the stream (0 = StdOut).
pub(in crate::target::shared::code) fn lower_process_poll_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_from: bool,
) -> HelperResult {
    const AVAIL: usize = 0x30;
    const FILE: usize = 0x38;
    const MS: usize = 0x40;
    const DEADLINE: usize = 0x48;
    const FD: usize = 0x50;
    const FROM: usize = 0x58;
    const FRAME: usize = 0x60;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let poll_loop = format!("{symbol}_poll_loop");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::store_u64(abi::mfb_arg(1), sp, MS),
    ];
    if with_from {
        instructions.push(abi::store_u64(abi::mfb_arg(2), sp, FROM));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), sp, FROM),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        // deadline = GetTickCount64() + ms
    ]);
    platform.emit_libc_call(
        "GetTickCount64",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, MS),
        abi::add_registers(abi::mfb_arg(0), abi::c_return(0), abi::mfb_arg(1)),
        abi::store_u64(abi::mfb_arg(0), sp, DEADLINE),
        abi::label(&poll_loop),
        // PeekNamedPipe(fd, NULL, 0, NULL, &avail, NULL)
        abi::store_u64(abi::ZERO, sp, AVAIL),
        abi::add_immediate(abi::mfb_arg(0), sp, AVAIL),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th &avail
        abi::store_u64(abi::ZERO, sp, 0x28),       // 6th NULL
        abi::load_u64(abi::mfb_arg(0), sp, FD),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "PeekNamedPipe",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&ready), // FALSE = broken pipe → readable (EOF drain)
        abi::load_u32(abi::mfb_arg(0), sp, AVAIL),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_ne(&ready),
    ]);
    // Not ready yet: past the deadline? (now >= deadline → timeout).
    platform.emit_libc_call(
        "GetTickCount64",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, DEADLINE),
        abi::compare_registers(abi::c_return(0), abi::mfb_arg(1)),
        abi::branch_ge(&not_ready),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
    ]);
    platform.emit_libc_call("Sleep", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::branch(&poll_loop),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_process_signal_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("signal")
}

pub(in crate::target::shared::code) fn lower_process_didsignal_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("didSignal")
}

pub(in crate::target::shared::code) fn lower_process_detach_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("detach")
}
// ---------------------------------------------------------------------------
// process.spawn (Windows) — CreateProcessA with the child inheriting the
// parent's console (no stdio pipes in this first slice; pipe redirection lands
// with the I/O phase). Builds a space-joined command line from the argv list.
// Record: hProcess@8, pid@64, exitcode@72, reaped@56; the fd slots stay 0.
// ---------------------------------------------------------------------------
pub(in crate::target::shared::code) fn lower_process_spawn_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    _with_env: bool,
) -> HelperResult {
    // Win64 call frame (all `sp`-relative, addressed at stack-adjust depth 1 —
    // this whole body runs inside one `subtract_stack(FRAME)`/`add_stack(FRAME)`
    // bracket so `finalize_frame` does NOT shift these offsets: the six outgoing
    // stack args must land at the *real* `sp+0x20..` the callee reads, and the
    // shadow/SI/PI/state slots must stay consistent with them. This mirrors the
    // fs `emit_build_argv_utf8` pattern — no abstract vregs, hence no spills that
    // would be shifted out from under the depth-1 accesses. State lives in the
    // slots below `PI`; `mfb_arg(0..3)` are transient scratch reloaded from the
    // slots after every helper call (`emit_alloc`/`emit_libc_call` clobber them).
    //   [0x00..0x20)  shadow space for callees
    //   [0x20..0x50)  CreateProcessA stack args 5..10
    //   [SI..SI+104)  STARTUPINFOA (dwFlags@60, hStdInput@80/hStdOutput@88/hStdError@96)
    //   [PI..PI+24)   PROCESS_INFORMATION (hProcess@0, hThread@8, dwProcessId@16)
    //   [SA..SA+24)   SECURITY_ATTRIBUTES (nLength@0, lpSD@8, bInheritHandle@16)
    //   IN_R/IN_W/OUT_R/OUT_W/ERR_R/ERR_W  CreatePipe out-handle slots
    //   LIST/N/DBASE/CMD/DP/IDX/VLENS/REC  scalar state slots
    const SI: usize = 0x50; // STARTUPINFOA (104 bytes)
    const SI_DWFLAGS: usize = 60;
    const SI_HSTDIN: usize = 80;
    const SI_HSTDOUT: usize = 88;
    const SI_HSTDERR: usize = 96;
    const PI: usize = 0xB8; // PROCESS_INFORMATION (24 bytes)
    const SA: usize = 0xD0; // SECURITY_ATTRIBUTES (24 bytes)
    const IN_R: usize = 0xE8; // child stdin read end (child inherits)
    const IN_W: usize = 0xF0; // parent stdin write end (kept)
    const OUT_R: usize = 0xF8; // parent stdout read end (kept)
    const OUT_W: usize = 0x100; // child stdout write end (child inherits)
    const ERR_R: usize = 0x108; // parent stderr read end (kept)
    const ERR_W: usize = 0x110; // child stderr write end (child inherits)
    const LIST: usize = 0x118; // argv list pointer
    const N: usize = 0x120; // argv count
    const DBASE: usize = 0x128; // string data base
    const CMD: usize = 0x130; // cmdline buffer (also the running length before alloc)
    const DP: usize = 0x138; // cmdline write cursor
    const IDX: usize = 0x140; // outer argv index
    const VLENS: usize = 0x148; // current arg byte-length
    const REC: usize = 0x150; // resource record pointer
    const FRAME: usize = 0x160; // 16-aligned
    const HANDLE_FLAG_INHERIT: &str = "1";
    const STARTF_USESTDHANDLES: &str = "256"; // 0x100
    const LIST_COUNT: usize = COLLECTION_OFFSET_COUNT;
    const LIST_CAP: usize = COLLECTION_OFFSET_CAPACITY;
    const HDR: usize = COLLECTION_HEADER_SIZE;
    const ENT: usize = COLLECTION_ENTRY_SIZE;
    const VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
    const VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;
    let sp = abi::stack_pointer();

    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let spawn_fail = format!("{symbol}_spawn_fail");
    let sum_loop = format!("{symbol}_sum_loop");
    let sum_done = format!("{symbol}_sum_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let inner_loop = format!("{symbol}_inner_loop");
    let inner_done = format!("{symbol}_inner_done");
    let no_space = format!("{symbol}_no_space");
    let done = format!("{symbol}_done");

    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        // The argv list pointer arrives in the return register.
        abi::store_u64(abi::return_register(), sp, LIST),
        abi::load_u64(abi::mfb_arg(0), sp, LIST),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), LIST_COUNT),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_eq(&invalid),
        abi::store_u64(abi::mfb_arg(1), sp, N),
        // dbase = list + cap*ENT + HDR
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), LIST_CAP),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
        abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(2), sp, DBASE),
        // running length = n (separators + NUL) + sum(vlen); stash in CMD slot.
        abi::store_u64(abi::mfb_arg(1), sp, CMD),
        abi::store_u64(abi::ZERO, sp, IDX),
        abi::label(&sum_loop),
        abi::load_u64(abi::mfb_arg(0), sp, IDX),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&sum_done),
        // entry = list + idx*ENT + HDR
        abi::load_u64(abi::mfb_arg(2), sp, LIST),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
        abi::multiply_registers(abi::mfb_arg(1), abi::mfb_arg(0), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), HDR),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(1)),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(1), VLEN),
        abi::load_u64(abi::mfb_arg(2), sp, CMD),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(1)),
        abi::store_u64(abi::mfb_arg(2), sp, CMD),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, IDX),
        abi::branch(&sum_loop),
        abi::label(&sum_done),
        // cmd = arena_alloc(len + 1, align 1)
        abi::load_u64(abi::return_register(), sp, CMD),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ];
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, CMD),
        abi::store_u64(abi::mfb_return(1), sp, DP),
        abi::store_u64(abi::ZERO, sp, IDX),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, IDX),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&copy_done),
        // separator space before every arg but the first
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&no_space),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "32"),
        abi::load_u64(abi::mfb_arg(3), sp, DP),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(3), 0),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::store_u64(abi::mfb_arg(3), sp, DP),
        abi::label(&no_space),
        // entry = list + idx*ENT + HDR
        abi::load_u64(abi::mfb_arg(2), sp, LIST),
        abi::load_u64(abi::mfb_arg(0), sp, IDX),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
        abi::multiply_registers(abi::mfb_arg(1), abi::mfb_arg(0), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), HDR),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(1)),
        // vlen -> VLENS slot; srcp -> mfb_arg(0); dp -> mfb_arg(1)
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(1), VLEN),
        abi::store_u64(abi::mfb_arg(2), sp, VLENS),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), VOFF),
        abi::load_u64(abi::mfb_arg(2), sp, DBASE),
        abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(2), abi::mfb_arg(0)),
        abi::load_u64(abi::mfb_arg(1), sp, DP),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // j
        abi::label(&inner_loop),
        abi::load_u64(abi::mfb_arg(2), sp, VLENS),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&inner_done),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::branch(&inner_loop),
        abi::label(&inner_done),
        abi::store_u64(abi::mfb_arg(1), sp, DP),
        abi::load_u64(abi::mfb_arg(0), sp, IDX),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, IDX),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::load_u64(abi::mfb_arg(1), sp, DP),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0), // NUL-terminate
        // SECURITY_ATTRIBUTES{ nLength = 24, lpSecurityDescriptor = NULL,
        // bInheritHandle = TRUE } — both pipe ends inheritable, then the parent
        // end of each is stripped of inheritance via SetHandleInformation.
        abi::move_immediate(abi::mfb_arg(0), "Integer", "24"),
        abi::store_u32(abi::mfb_arg(0), sp, SA),
        abi::store_u64(abi::ZERO, sp, SA + 8),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u32(abi::mfb_arg(0), sp, SA + 16),
    ]);
    // Three anonymous pipes: stdin (parent writes IN_W → child reads IN_R),
    // stdout (child writes OUT_W → parent reads OUT_R), stderr (ERR_W/ERR_R).
    // CreatePipe(&read, &write, &sa, 0); on FALSE → spawn_fail.
    for (read_slot, write_slot) in [(IN_R, IN_W), (OUT_R, OUT_W), (ERR_R, ERR_W)] {
        instructions.extend([
            abi::add_immediate(abi::mfb_arg(0), sp, read_slot),
            abi::add_immediate(abi::mfb_arg(1), sp, write_slot),
            abi::add_immediate(abi::mfb_arg(2), sp, SA),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        ]);
        platform.emit_libc_call(
            "CreatePipe",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&spawn_fail),
        ]);
    }
    // Strip inheritance from each parent-held end (IN_W/OUT_R/ERR_R) so the child
    // does not hold a duplicate that would keep a pipe from reaching EOF.
    for parent_slot in [IN_W, OUT_R, ERR_R] {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), sp, parent_slot),
            abi::move_immediate(abi::mfb_arg(1), "Integer", HANDLE_FLAG_INHERIT),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
        ]);
        platform.emit_libc_call(
            "SetHandleInformation",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    // Zero STARTUPINFOA (104 bytes), set cb = 104, dwFlags = STARTF_USESTDHANDLES,
    // and the three child-end handles.
    for off in (0..104).step_by(8) {
        instructions.push(abi::store_u64(abi::ZERO, sp, SI + off));
    }
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(0), "Integer", "104"),
        abi::store_u32(abi::mfb_arg(0), sp, SI),
        abi::move_immediate(abi::mfb_arg(0), "Integer", STARTF_USESTDHANDLES),
        abi::store_u32(abi::mfb_arg(0), sp, SI + SI_DWFLAGS),
        abi::load_u64(abi::mfb_arg(0), sp, IN_R),
        abi::store_u64(abi::mfb_arg(0), sp, SI + SI_HSTDIN),
        abi::load_u64(abi::mfb_arg(0), sp, OUT_W),
        abi::store_u64(abi::mfb_arg(0), sp, SI + SI_HSTDOUT),
        abi::load_u64(abi::mfb_arg(0), sp, ERR_W),
        abi::store_u64(abi::mfb_arg(0), sp, SI + SI_HSTDERR),
        // CreateProcessA(NULL, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi).
        // Win64: register args 0..3 in mfb_arg (rcx/rdx/r8/r9); stack args 5..10
        // stored directly at sp+0x20.. (after the 32-byte shadow).
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th bInheritHandles = TRUE
        abi::store_u64(abi::ZERO, sp, 0x28),       // 6th dwCreationFlags
        abi::store_u64(abi::ZERO, sp, 0x30),       // 7th lpEnvironment
        abi::store_u64(abi::ZERO, sp, 0x38),       // 8th lpCurrentDirectory
        abi::add_immediate(abi::mfb_arg(0), sp, SI),
        abi::store_u64(abi::mfb_arg(0), sp, 0x40), // 9th &si
        abi::add_immediate(abi::mfb_arg(0), sp, PI),
        abi::store_u64(abi::mfb_arg(0), sp, 0x48), // 10th &pi
        // A register arg is zeroed with an immediate, NOT `move_register(_, ZERO)`:
        // x86-64 has no hardware zero register, so `ZERO` maps to a GPR holding
        // garbage (only `store_*` special-cases it to an immediate 0).
        abi::move_immediate(abi::mfb_arg(0), "Integer", "0"), // lpApplicationName NULL
        abi::load_u64(abi::mfb_arg(1), sp, CMD),              // lpCommandLine
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"), // lpProcessAttributes NULL
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // lpThreadAttributes NULL
    ]);
    platform.emit_libc_call(
        "CreateProcessA",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&spawn_fail),
    ]);
    // Close the child-end handles the parent no longer needs + the thread handle.
    for close_slot in [PI + 8, IN_R, OUT_W, ERR_W] {
        instructions.push(abi::load_u64(abi::mfb_arg(0), sp, close_slot));
        platform.emit_libc_call(
            "CloseHandle",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    // Allocate + stamp the record.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, REC),
        abi::load_u64(abi::mfb_arg(0), sp, REC),
        abi::move_immediate(abi::mfb_arg(1), "Integer", RESOURCE_TAG_PROCESS),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_TAG),
        abi::load_u64(abi::mfb_arg(1), sp, PI), // hProcess
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_STATE),
        abi::load_u64(abi::mfb_arg(1), sp, IN_W),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDIN_W),
        abi::load_u64(abi::mfb_arg(1), sp, OUT_R),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
        abi::load_u64(abi::mfb_arg(1), sp, ERR_R),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_REAPED),
        abi::load_u32(abi::mfb_arg(1), sp, PI + 16), // dwProcessId
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STATUS), // pid cached here on Windows
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_EXITCODE),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), 80),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), 88),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_arg(0)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&spawn_fail),
    ]);
    win_fail(
        symbol,
        ERR_SPAWN_FAILED_CODE,
        ERR_SPAWN_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    win_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    win_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    // Every path funnels here; unwind the frame before returning.
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_process_pid_helper(
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
    win_fail(
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

pub(in crate::target::shared::code) fn lower_process_waitfor_helper(
    symbol: &str,
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
    win_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_process_drop_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Explicit Win64 frame (depth-1, no vregs): shadow [0x00..0x20) + FILE slot
    // (the record pointer, live across TerminateProcess/CloseHandle). The shadow
    // reservation is mandatory (see `lower_process_waitfor_helper`).
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let done = format!("{symbol}_done");
    let done_ok = format!("{symbol}_done_ok");
    let already = format!("{symbol}_already");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&done_ok),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_REAPED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&already),
        // TerminateProcess(hProcess, 1)
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ];
    platform.emit_libc_call(
        "TerminateProcess",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&already));
    // CloseHandle(hProcess)
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
    ]);
    platform.emit_libc_call(
        "CloseHandle",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::label(&done_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::add_stack(FRAME),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
