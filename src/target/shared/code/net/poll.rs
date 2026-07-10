//! Native code generation for the `net` package poll/timeout helpers:
//! `net.poll` readiness checks and `net.setReadTimeout`/`net.setWriteTimeout`
//! socket-option machinery. See the parent module for the shared emitters.

use std::collections::HashMap;

use super::*;

// ---------------------------------------------------------------------------
// net.poll
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_poll_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2): the `pollfd` is an explicit on-stack
    // local; scratch is vregs the allocator places.
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 16;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let poll_fail = format!("{symbol}_poll_fail");
    let not_ready = format!("{symbol}_not_ready");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        // x1 = timeoutMs; reject negative timeouts.
        abi::compare_immediate("x1", "0"),
        abi::branch_lt(&invalid),
        abi::move_register("%v12", "x1"),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        // pollfd { int fd; short events = POLLIN; short revents; }
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("%v10", "Integer", POLLIN),
        abi::store_u8("%v10", abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        // poll(&pollfd, 1, timeoutMs)
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x1", "Integer", "1"),
        abi::move_register("x2", "%v12"),
    ]);
    platform.emit_libc_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
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
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
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
        // x1 = timeoutMs; reject negatives.
        abi::compare_immediate("x1", "0"),
        abi::branch_lt(&invalid),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
        abi::move_immediate("%v10", "Integer", "1000"),
        abi::unsigned_divide_registers("%v11", "x1", "%v10"),
        abi::multiply_subtract_registers("%v12", "%v11", "%v10", "x1"),
        abi::move_immediate("%v13", "Integer", "1000"),
        abi::multiply_registers("%v12", "%v12", "%v13"),
        abi::store_u64("%v11", abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64("%v12", abi::stack_pointer(), TIMEVAL_OFFSET + 8),
        // setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO/SO_SNDTIMEO, &tv, 16)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", platform.sol_socket()),
        abi::move_immediate(
            "x2",
            "Integer",
            if write {
                platform.so_sndtimeo()
            } else {
                platform.so_rcvtimeo()
            },
        ),
        abi::add_immediate("x3", abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::move_immediate("x4", "Integer", "16"),
    ]);
    platform.emit_libc_call(
        "setsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
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
