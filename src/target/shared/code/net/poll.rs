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
    let poll_infinite = format!("{symbol}_poll_infinite");
    let poll_fail = format!("{symbol}_poll_fail");
    let not_ready = format!("{symbol}_not_ready");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        // plan-73-C: an OMITTED timeout is padded with the unbounded sentinel
        // (i64::MIN) → block until readable, i.e. poll() with a -1 timeout. Any
        // other negative value is rejected; a non-negative value is clamped below.
        abi::move_immediate("%v13", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(1), "%v13"),
        abi::branch_eq(&poll_infinite),
        // x1 = timeoutMs; reject negative timeouts.
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_lt(&invalid),
        abi::move_register("%v12", abi::c_arg(1)),
        // Clamp timeoutMs to INT_MAX: poll() takes a C `int`, so a 64-bit value
        // with bit 31 set would be read as a negative timeout (block forever)
        // instead of a long wait (bug-239). Negatives were already rejected above.
        abi::move_immediate("%v13", "Integer", "2147483647"),
        abi::compare_registers("%v12", "%v13"),
        abi::branch_le(&timeout_ok),
        abi::move_register("%v12", "%v13"),
        abi::branch(&timeout_ok),
        // Unbounded (omit) form: -1 makes poll() block until the socket is readable.
        abi::label(&poll_infinite),
        abi::bitwise_not("%v12", abi::ZERO),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), "%v12"),
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
        ("%v9").into(),
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
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// net.pollList  (plan-76-A: net::poll(List OF RES Socket) AS Socket)
// ---------------------------------------------------------------------------

/// The readiness multiplex: `poll(socks AS List OF RES Socket[, timeoutMs]) AS
/// Socket`. Blocks until at least one socket in `socks` is readable, then returns a
/// BORROWED pointer to the first ready one (lowest list index) — the list retains
/// ownership and closes each socket exactly once on scope exit (§15.6). An empty
/// list raises `ErrInvalidArgument`; expiry with none ready raises `ErrTimeout`
/// (this is a *producing* call, per the plan-73-A classification).
///
/// The generalization of `lower_net_poll_helper` to N fds: it builds a transient
/// `pollfd[n]` in the arena (a `List OF RES Socket` is a kind-0 collection, so `n`
/// is a runtime value), issues one `poll(2)`/`WSAPoll` over the whole array reusing
/// the scalar helper's sentinel/clamp/EINTR-retry policy verbatim, scans `revents`
/// for the first ready slot, and returns that element's record pointer. The array is
/// `arena_free`d on every exit path that allocated it (no leak across the loop).
///
/// `socks` (the collection pointer) arrives in `x0`; `timeoutMs` in `x1`.
pub(in crate::target::shared::code) fn lower_net_poll_list_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Explicit sp-relative locals that must survive the arena_alloc / poll /
    // arena_free calls (each clobbers all caller-saved registers). The pollfd
    // *array* lives in the arena, not here; these hold only scalars.
    const TIMEOUT_OFF: usize = 0;
    const COUNT_OFF: usize = 8;
    const COLL_OFF: usize = 16;
    const SIZE_OFF: usize = 24;
    const BUF_OFF: usize = 32;
    const DATABASE_OFF: usize = 40;
    const RESULT_OFF: usize = 48;
    const FRAME_SIZE: usize = 64;

    // Windows `WSAPOLLFD` is `{ SOCKET fd(8); SHORT events; SHORT revents }` — an
    // 8-byte fd, so the struct is 16 bytes and `revents` sits at +10; POSIX `struct
    // pollfd` is `{ int fd; short events; short revents }` — 8 bytes, `revents` at
    // +6 (mirrors `emit_pollfd_events`).
    let windows = platform.family() == PlatformFamily::Windows;
    let pollfd_stride: usize = if windows { 16 } else { 8 };
    let revents_off: usize = if windows { 10 } else { 6 };

    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let poll_infinite = format!("{symbol}_poll_infinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let poll_retry = format!("{symbol}_poll_retry");
    let poll_fail = format!("{symbol}_poll_fail");
    let expiry = format!("{symbol}_expiry");
    let scan_loop = format!("{symbol}_scan_loop");
    let found = format!("{symbol}_found");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    // --- Normalize the timeout (identical policy to the scalar helper) ---
    instructions.extend([
        abi::move_immediate("%v13", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(1), "%v13"),
        abi::branch_eq(&poll_infinite),
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_lt(&invalid),
        abi::move_register("%v12", abi::c_arg(1)),
        abi::move_immediate("%v13", "Integer", "2147483647"),
        abi::compare_registers("%v12", "%v13"),
        abi::branch_le(&timeout_ok),
        abi::move_register("%v12", "%v13"),
        abi::branch(&timeout_ok),
        abi::label(&poll_infinite),
        abi::bitwise_not("%v12", abi::ZERO), // -1 → block until readable
        abi::label(&timeout_ok),
        abi::store_u64("%v12", abi::stack_pointer(), TIMEOUT_OFF),
        // Capture the collection pointer (x0) before any call clobbers it.
        abi::move_register("%v9", abi::return_register()),
        abi::store_u64("%v9", abi::stack_pointer(), COLL_OFF),
        // count = socks.count; reject the empty list.
        abi::load_u64("%v10", "%v9", COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&invalid),
        abi::store_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        // data_base = socks + HEADER + capacity * ENTRY_SIZE (kind-0 list; a
        // resource element is a bare pointer stored via the entry table).
        abi::load_u64("%v11", "%v9", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v11", "%v13"),
        abi::add_immediate("%v14", "%v9", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v14", "%v14", "%v11"),
        abi::store_u64("%v14", abi::stack_pointer(), DATABASE_OFF),
        // size = count * pollfd_stride; arena_alloc(size, 8).
        abi::move_immediate("%v13", "Integer", &pollfd_stride.to_string()),
        abi::multiply_registers("%v15", "%v10", "%v13"),
        abi::store_u64("%v15", abi::stack_pointer(), SIZE_OFF),
        abi::move_register(abi::return_register(), "%v15"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFF),
        // --- Fill pollfd[i] from each socket's fd (i in a vreg; no call in loop) ---
        abi::move_immediate("%v8", "Integer", "0"),
        abi::label(&fill_loop),
        abi::load_u64("%v11", abi::stack_pointer(), COUNT_OFF),
        abi::compare_registers("%v8", "%v11"),
        abi::branch_ge(&fill_done),
        // entry_ptr = socks + HEADER + i * ENTRY_SIZE
        abi::load_u64("%v9", abi::stack_pointer(), COLL_OFF),
        abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v8", "%v13"),
        abi::add_immediate("%v9", "%v9", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v9", "%v9", "%v14"),
        // value_offset → element address → socket record ptr → fd
        abi::load_u64("%v9", "%v9", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::load_u64("%v14", abi::stack_pointer(), DATABASE_OFF),
        abi::add_registers("%v9", "%v14", "%v9"),
        abi::load_u64("%v9", "%v9", 0),
        abi::load_u64("%v9", "%v9", FILE_OFFSET_FD),
        // pfd_i = buf + i * pollfd_stride
        abi::load_u64("%v11", abi::stack_pointer(), BUF_OFF),
        abi::move_immediate("%v13", "Integer", &pollfd_stride.to_string()),
        abi::multiply_registers("%v14", "%v8", "%v13"),
        abi::add_registers("%v11", "%v11", "%v14"),
        // fd (8 bytes; upper 32 zero → also clears events/revents on POSIX).
        abi::store_u64("%v9", "%v11", 0),
    ]);
    // events = POLLIN / POLLRDNORM, revents = 0, written relative to pfd_i (%v11).
    if windows {
        instructions.extend([
            abi::store_u8(abi::ZERO, "%v11", 8),
            abi::move_immediate("%v10", "Integer", "1"),
            abi::store_u8("%v10", "%v11", 9), // POLLRDNORM = 0x0100
            abi::store_u8(abi::ZERO, "%v11", 10),
            abi::store_u8(abi::ZERO, "%v11", 11),
        ]);
    } else {
        instructions.extend([
            abi::move_immediate("%v10", "Integer", POLLIN),
            abi::store_u8("%v10", "%v11", 4),
            abi::store_u8(abi::ZERO, "%v11", 5),
            abi::store_u8(abi::ZERO, "%v11", 6),
            abi::store_u8(abi::ZERO, "%v11", 7),
        ]);
    }
    instructions.extend([
        abi::add_immediate("%v8", "%v8", 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        // --- poll(buf, count, timeout); EINTR-retry (bug-115) ---
        abi::label(&poll_retry),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), BUF_OFF),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), COUNT_OFF),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFF),
    ]);
    platform.emit_libc_call(
        net_symbol(platform, NetSymbol::Poll),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return — sign-extend before the signed compares (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_fail),
        abi::branch_eq(&expiry),
        // ret > 0: at least one fd is ready. Scan revents for the first set slot.
        abi::move_immediate("%v8", "Integer", "0"),
        abi::label(&scan_loop),
        abi::load_u64("%v11", abi::stack_pointer(), COUNT_OFF),
        abi::compare_registers("%v8", "%v11"),
        abi::branch_ge(&expiry), // defensive: ret>0 but no slot found → treat as expiry
        abi::load_u64("%v9", abi::stack_pointer(), BUF_OFF),
        abi::move_immediate("%v13", "Integer", &pollfd_stride.to_string()),
        abi::multiply_registers("%v14", "%v8", "%v13"),
        abi::add_registers("%v9", "%v9", "%v14"),
        abi::load_u16("%v15", "%v9", revents_off),
        abi::compare_immediate("%v15", "0"),
        abi::branch_ne(&found),
        abi::add_immediate("%v8", "%v8", 1),
        abi::branch(&scan_loop),
        abi::label(&found),
        // Recompute socks[i]'s record ptr (i in %v8) — the borrowed result value.
        abi::load_u64("%v9", abi::stack_pointer(), COLL_OFF),
        abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v8", "%v13"),
        abi::add_immediate("%v9", "%v9", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v9", "%v9", "%v14"),
        abi::load_u64("%v9", "%v9", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::load_u64("%v14", abi::stack_pointer(), DATABASE_OFF),
        abi::add_registers("%v9", "%v14", "%v9"),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), RESULT_OFF),
        // arena_free(buf, size) before returning (no leak).
        abi::load_u64(abi::return_register(), abi::stack_pointer(), BUF_OFF),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SIZE_OFF),
        abi::branch_link(ARENA_FREE_SYMBOL),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // One internal-call relocation covers all `arena_free` sites in this helper
    // (the reloc is a per-(from,to) declaration, not per-instruction — mirrors
    // io_stdin.rs's arena_free wiring).
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    // Expiry: poll timed out with none ready. Free the array, raise ErrTimeout.
    instructions.push(abi::label(&expiry));
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), BUF_OFF),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SIZE_OFF),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // Poll returned < 0: EINTR → re-issue; any other errno → hard failure. Read the
    // real errno (poll goes through libc).
    instructions.push(abi::label(&poll_fail));
    platform.emit_errno(
        symbol,
        ("%v9").into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", EINTR_ERRNO),
        abi::branch_eq(&poll_retry),
        // Hard error: free the array, then report resource-closed (a stale/closed fd
        // in the set is the realistic cause), matching the scalar helper's class.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), BUF_OFF),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SIZE_OFF),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // Empty list or negative timeout — rejected before any allocation.
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // Arena allocation failed.
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
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
    let nb_ok = format!("{symbol}_nb_ok");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        // timeoutMs arrives in the incoming-arg register; copy it to an
        // allocator-placed vreg (plan-34-B Phase 3) so the tv math below is not
        // pinned to a physical register. Reject negatives.
        abi::move_register("%v14", abi::c_arg(1)),
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
        instructions.extend([
            abi::store_u64("%v14", abi::stack_pointer(), TIMEVAL_OFFSET),
            // plan-73-C: `timeoutMs == 0` now means NON-BLOCKING (immediate
            // `ErrTimeout` when not ready), not "disable". Winsock SO_*TIMEO of 0 is
            // infinite, so use the smallest expressible wait (1 ms) for the 0 case.
            abi::compare_immediate("%v14", "0"),
            abi::branch_ne(&nb_ok),
            abi::move_immediate("%v13", "Integer", "1"),
            abi::store_u64("%v13", abi::stack_pointer(), TIMEVAL_OFFSET),
            abi::label(&nb_ok),
        ]);
        "4"
    } else {
        instructions.extend([
            // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
            abi::move_immediate("%v10", "Integer", "1000"),
            abi::unsigned_divide_registers("%v11", "%v14", "%v10"),
            abi::multiply_subtract_registers("%v12", "%v11", "%v10", "%v14"),
            abi::move_immediate("%v13", "Integer", "1000"),
            abi::multiply_registers("%v12", "%v12", "%v13"),
            // plan-73-C: `timeoutMs == 0` now means NON-BLOCKING, not "disable". A
            // POSIX SO_*TIMEO of {0,0} is infinite, so use the smallest wait (1 µs =
            // tv_usec 1) for the 0 case — a near-immediate `ErrTimeout` when not ready.
            abi::compare_immediate("%v14", "0"),
            abi::branch_ne(&nb_ok),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::label(&nb_ok),
            abi::store_u64("%v11", abi::stack_pointer(), TIMEVAL_OFFSET),
            abi::store_u64("%v12", abi::stack_pointer(), TIMEVAL_OFFSET + 8),
        ]);
        "16"
    };
    instructions.extend([
        // setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO/SO_SNDTIMEO, &optval, optval_len)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
        abi::move_immediate(
            abi::c_arg(2),
            "Integer",
            if write {
                platform.so_sndtimeo()
            } else {
                platform.so_rcvtimeo()
            },
        ),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::move_immediate(abi::c_arg(4), "Integer", optval_len),
    ]);
    // setsockopt has FIVE int args; on Win64 optlen (the 5th) is a stack argument
    // above the shadow, not rdi (bug-384) — a garbage optlen makes
    // SO_RCVTIMEO/SNDTIMEO setsockopt fail. The shared helper spills it through
    // the outgoing-args sentinel; POSIX passes it in a register, byte-unchanged.
    super::super::native_helpers::emit_external_int_call(
        platform,
        net_symbol(platform, NetSymbol::SetSockOpt),
        symbol,
        5,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
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
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}
