//! Native code generation for the `net` package poll/timeout helpers:
//! `net.poll` readiness checks and `net.setReadTimeout`/`net.setWriteTimeout`
//! socket-option machinery. See the parent module for the shared emitters.

use std::collections::HashMap;

use super::*;

// `EINTR_ERRNO` (bug-115) is defined once in `net/mod.rs` and reaches here via
// the `use super::*` glob above; this module previously shadowed it with a
// byte-identical local copy (bug-331 §I).

// ---------------------------------------------------------------------------
// net.poll
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_poll_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2): the `pollfd` is an explicit on-stack
    // local; scratch is vregs the allocator places.
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 16;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let poll_retry = format!("{symbol}_poll_retry");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let poll_fail = format!("{symbol}_poll_fail");
    let not_ready = format!("{symbol}_not_ready");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        // x1 = timeoutMs; reject negative timeouts.
        abi::compare_immediate(abi::ARG[1], "0"),
        abi::branch_lt(&invalid),
        abi::move_register("%v12", abi::ARG[1]),
        // Clamp timeoutMs to INT_MAX: poll() takes a C `int`, so a 64-bit value
        // with bit 31 set would be read as a negative timeout (block forever)
        // instead of a long wait (bug-239). Negatives were already rejected above.
        abi::move_immediate("%v13", "Integer", "2147483647"),
        abi::compare_registers("%v12", "%v13"),
        abi::branch_le(&timeout_ok),
        abi::move_register("%v12", "%v13"),
        abi::label(&timeout_ok),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        // pollfd { int fd; short events = POLLIN; short revents; }
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD_OFFSET),
    ]);
    emit_pollfd_events(platform, POLLFD_OFFSET, &mut instructions);
    instructions.extend([
        // poll(&pollfd, 1, timeoutMs); poll_retry re-issues the call (the pollfd is
        // already on the stack and %v12 holds the timeout) on an EINTR (bug-115).
        abi::label(&poll_retry),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::move_register(abi::ARG[2], "%v12"),
    ]);
    platform.emit_libc_call(
        net_symbol(platform, NetSymbol::Poll),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return (poll) — sign-extend before the signed compares; a -1 read
        // as large-positive would skip poll_fail/not_ready and fall through to
        // "socket ready" (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_fail),
        abi::branch_eq(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_fail),
    ]);
    // bug-115: a signal that interrupts poll returns -1/EINTR; re-issue rather
    // than reporting a spurious resource-closed failure. poll goes through libc,
    // so read the real code from errno.
    platform.emit_errno(
        symbol,
        "%v9",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", EINTR_ERRNO),
        abi::branch_eq(&poll_retry),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
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
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// net.setReadTimeout / net.setWriteTimeout
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_set_timeout_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    write: bool,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2): the `timeval` is an explicit on-stack
    // local; scratch is vregs.
    const FRAME_SIZE: usize = 48;
    const FD_OFFSET: usize = 8;
    const TIMEVAL_OFFSET: usize = 16; // tv_sec (8) + tv_usec (8)

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let set_fail = format!("{symbol}_set_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        // timeoutMs arrives in the incoming-arg register; copy it to an
        // allocator-placed vreg (plan-34-B Phase 3) so the tv math below is not
        // pinned to a physical register. Reject negatives.
        abi::move_register("%v14", abi::ARG[1]),
        abi::compare_immediate("%v14", "0"),
        abi::branch_lt(&invalid),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
    ]);
    // Winsock SO_RCVTIMEO/SO_SNDTIMEO optval is a DWORD of milliseconds, not a
    // struct timeval; store the raw ms and pass 4 bytes (plan-47-I). POSIX builds
    // the timeval, byte-identical to the pre-seam sequence.
    let win_timeout = platform.family() == PlatformFamily::Windows;
    let optval_len = if win_timeout {
        instructions.push(abi::store_u64("%v14", abi::stack_pointer(), TIMEVAL_OFFSET));
        "4"
    } else {
        instructions.extend([
            // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
            abi::move_immediate("%v10", "Integer", "1000"),
            abi::unsigned_divide_registers("%v11", "%v14", "%v10"),
            abi::multiply_subtract_registers("%v12", "%v11", "%v10", "%v14"),
            abi::move_immediate("%v13", "Integer", "1000"),
            abi::multiply_registers("%v12", "%v12", "%v13"),
            abi::store_u64("%v11", abi::stack_pointer(), TIMEVAL_OFFSET),
            abi::store_u64("%v12", abi::stack_pointer(), TIMEVAL_OFFSET + 8),
        ]);
        "16"
    };
    instructions.extend([
        // setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO/SO_SNDTIMEO, &optval, optval_len)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", platform.sol_socket()),
        abi::move_immediate(
            abi::ARG[2],
            "Integer",
            if write {
                platform.so_sndtimeo()
            } else {
                platform.so_rcvtimeo()
            },
        ),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::move_immediate(abi::ARG[4], "Integer", optval_len),
    ]);
    if win_timeout {
        // setsockopt has FIVE args; on Win64 optlen (the 5th) is a stack argument
        // above the shadow space, not rdi (bug-384). Without this, a garbage optlen
        // makes SO_RCVTIMEO/SNDTIMEO setsockopt fail (SO_REUSEADDR tolerates it, so
        // TCP was unaffected).
        instructions.extend([
            abi::subtract_stack(0x30),
            abi::store_u64(abi::ARG[4], abi::stack_pointer(), 0x20),
        ]);
    }
    platform.emit_libc_call(
        net_symbol(platform, NetSymbol::SetSockOpt),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if win_timeout {
        instructions.push(abi::add_stack(0x30));
    }
    instructions.extend([
        // `setsockopt` returns a C `int`, and both AAPCS and SysV leave the upper
        // 32 bits of the return register unspecified (bug-310, the bug-170 class).
        // Without this, a `-1` whose upper bits happen to be clear reads as
        // +4294967295, `branch_lt` is not taken, and the failure falls through to
        // the success path — the caller believes the timeout is armed when it is
        // not, and a later blocking read/write never times out. Every other
        // int-returning libc call in the net layer sign-extends before its signed
        // compare; this site was missed.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&set_fail),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&set_fail),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
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
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}
