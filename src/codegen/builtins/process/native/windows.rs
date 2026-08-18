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

// --- codegen tier imports (migration) ---
use super::*;
use crate::codegen::error::emission::emit_fail;
use crate::target::shared::abi;
use std::collections::HashMap;
pub(crate) fn unimplemented_on_windows(op: &str) -> HelperResult {
    Err(format!(
        "process::{op} native Windows backend is not yet emitted (plan-90-D)"
    ))
}
// process.close — close the parent's stdin write handle (signals the child's
// stdin EOF); mark it -1. Idempotent per-record via the -1 sentinel.

/// Emit a blocking `WriteFile(fd, [src_slot], [rem_slot], &[written_slot], NULL)`
/// loop that drains `rem` bytes from `src` on the pipe handle in `fd_slot`. The
/// `lpOverlapped` (5th, stack) arg goes at `sp+0x20`. On a `FALSE` return or a
/// zero-byte write (broken pipe) it branches to `fail`. `tag` disambiguates the
/// per-call labels (payload vs the trailing newline).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_write_all(
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
    platform.emit_libc_call(
        "WriteFile",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
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
pub(crate) fn lower_process_send_helper(
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
        symbol,
        "payload",
        sp,
        FD,
        SRCP,
        REM,
        WRITTEN,
        &closed_l,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
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
            symbol,
            "nl",
            sp,
            FD,
            SRCP,
            REM,
            WRITTEN,
            &closed_l,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
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
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
pub(crate) fn lower_process_drop_helper(
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
