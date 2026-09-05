//! Native code generation for `tcp`'s stream IO: accept, read, write.
//!
//! plan-110-E Phase 3 moved these out of `builtins/net/gen_io.rs`. They were
//! there because `net` used to own the stream; `tcp` owns it now, so the
//! emitters live with the package whose members they serve. What stayed behind
//! in `codegen::os::socket` is only what MORE THAN ONE package needs -- the
//! address builders, pollfd construction, the timeout setter.
//!
//! Each `lower_net_*_helper` emits a self-contained runtime function returning
//! the standard `(tag, value)` result. The names keep their `net_` infix: they
//! are the same emitters, and renaming them would churn every call site for no
//! behavioural gain.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::marshal::push_write_payload_view;
use crate::codegen::os::socket::shared::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// Winsock `WSAETIMEDOUT`: a blocking socket op that hits SO_RCVTIMEO/SO_SNDTIMEO
/// reports this on Windows, where POSIX reports EAGAIN/EWOULDBLOCK (plan-47-I,
/// bug-109). Used only on the `PlatformFamily::Windows` timeout arms.
const WSAETIMEDOUT: &str = "10060";

// ---------------------------------------------------------------------------
// net.accept
// ---------------------------------------------------------------------------

/// Restore a listener's pre-`accept` fcntl flags (bug-314 H2).
///
/// Emitted at each exit *before* the result/tag registers are set, never at the
/// shared `done` label. Two earlier attempts restored at `done` and both broke a
/// timed-out accept: the result is already established there, and this `fcntl` is a
/// call that destroys it -- the first reported success for a timeout, the second
/// segfaulted trying to spill around it. Restoring before the result exists removes
/// the conflict instead of working around it.
///
/// A no-op when `RESTORE_FLAGS_OFFSET` is 0, i.e. on the unbounded path, which never
/// went non-blocking. It also clears the flag, so a path crossing two restore sites
/// only issues the syscall once.
fn emit_listener_flags_restore(
    ctx: &mut EmitCtx,
    tag: &str,
    restore_flag_offset: usize,
    listener_fd_offset: usize,
    flags_offset: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let v9 = vregs.next();

    let skip = format!("{symbol}_restore_skip_{tag}");
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), restore_flag_offset),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&skip),
    ]);
    if platform.family() == PlatformFamily::Windows {
        // Winsock: ioctlsocket(listener, FIONBIO, &0). No flags word to restore.
        platform.emit_restore_blocking(
            listener_fd_offset,
            flags_offset,
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
    } else {
        ctx.instructions.extend([
            abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                listener_fd_offset,
            ),
            abi::move_immediate(abi::c_arg(1), "Integer", "4"), // F_SETFL
            abi::load_u64(abi::c_arg(2), abi::stack_pointer(), flags_offset),
        ]);
        platform.emit_variadic_external_call(
            net_symbol(platform, NetSymbol::Fcntl),
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
    }
    ctx.instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), restore_flag_offset),
        abi::label(&skip),
    ]);
    Ok(())
}

pub(crate) fn lower_net_accept_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<NetBodyParts, String> {
    const FRAME_SIZE: usize = 64;
    const FD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    // pollfd { int fd; short events; short revents } — 8 bytes for the bounded-wait
    // path (bug-185).
    const POLLFD_OFFSET: usize = 24;
    // bug-314 H2: the listener needs its own slot -- the success path overwrites
    // FD_OFFSET with the ACCEPTED socket's fd, so a restore keyed on FD_OFFSET
    // would set flags on the wrong descriptor.
    const LISTENER_FD_OFFSET: usize = 32;
    const FLAGS_OFFSET: usize = 40;
    const RESTORE_FLAGS_OFFSET: usize = 48;

    let closed = format!("{symbol}_closed");
    let accept_retry = format!("{symbol}_accept_retry");
    let accept_poll_retry = format!("{symbol}_accept_poll_retry");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let accept_ts_ok = format!("{symbol}_accept_ts_ok");
    let accept_invalid = format!("{symbol}_accept_invalid");
    let accept_fail = format!("{symbol}_accept_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        // plan-73-C timeout convention. Bounded wait (bug-185): poll(POLLIN) on the
        // listener before accepting so a caller-supplied deadline is honored. The
        // OMITTED overload pads the unbounded sentinel (i64::MIN) → the plain
        // block-forever accept below; `0` = one immediate attempt (poll with a 0
        // timeout → `ErrTimeout` when no client is pending); `> 0` = bounded (clamped
        // to INT_MAX, since poll takes a C `int`); any other negative =
        // `ErrInvalidArgument`.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RESTORE_FLAGS_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v11, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v10, &v11),
        abi::branch_eq(&accept_retry),
        abi::compare_immediate(&v10, "0"),
        abi::branch_lt(&accept_invalid),
        // Clamp a too-large timeout to INT_MAX for the poll below, then continue on
        // the bounded path (0 → an immediate, non-blocking poll).
        abi::move_immediate(&v11, "Integer", "2147483647"),
        abi::compare_registers(&v10, &v11),
        abi::branch_le(&accept_ts_ok),
        abi::move_register(&v10, &v11),
        abi::label(&accept_ts_ok),
        abi::store_u64(&v10, abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    // bug-314 H2: on the BOUNDED path only, put the listener in non-blocking mode.
    // The bug-185 wait polls POLLIN and then issues a *blocking* accept, so if the
    // one pending connection is aborted (RST/ECONNABORTED) or taken by another
    // thread between the poll and the accept, that accept waits for the NEXT client
    // and ignores timeoutMs entirely. Non-blocking turns that into EAGAIN, which
    // re-enters the poll against the deadline.
    //
    // `timeoutMs <= 0` is the deliberate block-forever overload and must not be
    // touched -- it branches to accept_retry above, before any of this.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), LISTENER_FD_OFFSET),
    ]);
    if platform.family() != PlatformFamily::Windows {
        // fcntl(fd, F_GETFL, 0) — read the flags emit_set_nonblocking OR-s below.
        // Winsock's ioctlsocket(FIONBIO) is stateless, so Windows skips the read.
        instructions.extend([
            abi::move_register(abi::return_register(), &v9),
            abi::move_immediate(abi::c_arg(1), "Integer", "3"), // F_GETFL
            abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        ]);
        platform.emit_variadic_external_call(
            net_symbol(platform, NetSymbol::Fcntl),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            FLAGS_OFFSET,
        ));
    }
    // fcntl(fd, F_SETFL, flags | O_NONBLOCK) — Windows: ioctlsocket(fd, FIONBIO, &1)
    platform.emit_set_nonblocking(
        LISTENER_FD_OFFSET,
        FLAGS_OFFSET,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), RESTORE_FLAGS_OFFSET),
        // poll(&pollfd { fd, POLLIN }, 1, timeoutMs); accept_poll_retry rebuilds the
        // pollfd and re-issues on EINTR (bug-115).
        abi::label(&accept_poll_retry),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
    ]);
    emit_pollfd_events(platform, POLLFD_OFFSET, &mut instructions, &mut vregs);
    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Poll),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return (poll) — sign-extend before the signed compares; a -1 read
        // as large-positive would skip the timeout/error branches (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&accept_timeout),
        abi::branch_gt(&accept_retry),
    ]);
    // A negative poll return is either EINTR (re-issue) or a genuine failure; poll
    // goes through libc here, so read the real code from errno (bug-115).
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&accept_poll_retry),
        abi::branch(&accept_fail),
        // accept(fd, NULL, NULL) — Linux `accept4(..., SOCK_CLOEXEC)` (bug-499);
        // accept_retry reloads fd from the stack so an EINTR retry re-issues the
        // identical call (bug-115).
        abi::label(&accept_retry),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    emit_accept_call(
        platform,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // A C `int` return (accept's fd) leaves x0[63:32] unspecified (bug-04/bug-170);
        // sign-extend before the signed relational compare so a -1 error isn't read as
        // a large-positive success.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    // macOS: the accepted socket gets FD_CLOEXEC here (no accept4) (bug-499).
    emit_fd_cloexec_fallback(
        platform,
        symbol,
        FD_OFFSET,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // The ACCEPTED socket must be blocking, whichever accept form produced it.
    //
    // The bounded path above put the LISTENER into non-blocking mode (bug-314 H2),
    // and on macOS/BSD `accept` hands the new socket the listener's file-status
    // flags — including O_NONBLOCK. Restoring the listener below does nothing for
    // the socket that was already created, so without this every read on a socket
    // from `accept(listener, timeoutMs)` returned EAGAIN, which the read helper
    // reports as ErrTimeout.
    //
    // It stayed hidden because it is data-dependent: with nothing between the
    // accept and the read, loopback bytes had usually already arrived, so the
    // non-blocking read found them. Any intervening work lost that race.
    //
    // FLAGS_OFFSET still holds the listener's ORIGINAL (blocking) flags —
    // `emit_set_nonblocking` reads that slot and ORs O_NONBLOCK into the register
    // it passes to fcntl without writing the slot back — so it is exactly the mode
    // the accepted socket should have. Guarded on RESTORE_FLAGS_OFFSET, which is
    // only set on the bounded path, so the block-forever overload is untouched and
    // stays byte-identical. Regression fixture:
    // `tests/rt-behavior/net/net-bounded-accept-blocking-rt`.
    {
        let v12 = vregs.next();
        let skip_accepted = format!("{symbol}_accepted_blocking_skip");
        instructions.extend([
            abi::load_u64(&v12, abi::stack_pointer(), RESTORE_FLAGS_OFFSET),
            abi::compare_immediate(&v12, "0"),
            abi::branch_eq(&skip_accepted),
        ]);
        if platform.family() == PlatformFamily::Windows {
            // Winsock: ioctlsocket(accepted, FIONBIO, &0). MSDN documents the
            // accepted socket as inheriting the listener's properties, so clear it
            // there too rather than relying on it not being inherited.
            platform.emit_restore_blocking(
                FD_OFFSET,
                FLAGS_OFFSET,
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        } else {
            instructions.extend([
                abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
                abi::move_immediate(abi::c_arg(1), "Integer", "4"), // F_SETFL
                abi::load_u64(abi::c_arg(2), abi::stack_pointer(), FLAGS_OFFSET),
            ]);
            platform.emit_variadic_external_call(
                net_symbol(platform, NetSymbol::Fcntl),
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.push(abi::label(&skip_accepted));
    }
    // bug-314 H2: the accepted fd is safely in FD_OFFSET and no result register
    // is live yet -- restore before emit_make_handle establishes the result.
    emit_listener_flags_restore(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "ok",
        RESTORE_FLAGS_OFFSET,
        LISTENER_FD_OFFSET,
        FLAGS_OFFSET,
        &mut vregs,
    )?;
    emit_make_handle(
        symbol,
        FD_OFFSET,
        RESOURCE_TAG_SOCKET,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &mut vregs,
    );
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&accept_fail),
    ]);
    // bug-115: a signal that interrupts the blocking accept returns -1/EINTR;
    // re-issue rather than reporting a spurious network failure. accept goes
    // through libc on every backend, so read the real code from errno.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&accept_retry),
        // bug-314 H2: with the listener non-blocking, EAGAIN means the connection
        // poll reported vanished before accept ran (aborted, or taken by another
        // thread). Re-enter the poll -- this is precisely the case that used to
        // block past the deadline. Like the EINTR edge above it re-polls with the
        // original timeout rather than the remainder, so a stream of racing aborts
        // can extend the wait; bounding the pathological case is what matters, and
        // each extension costs the peer another abort.
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_eq(&accept_poll_retry),
    ]);
    emit_listener_flags_restore(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "fail",
        RESTORE_FLAGS_OFFSET,
        LISTENER_FD_OFFSET,
        FLAGS_OFFSET,
        &mut vregs,
    )?;
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // No client arrived before the deadline (poll returned 0): report a timeout,
    // matching net::connectTcp's bounded-wait error (bug-185).
    instructions.push(abi::label(&accept_timeout));
    emit_listener_flags_restore(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "timeout",
        RESTORE_FLAGS_OFFSET,
        LISTENER_FD_OFFSET,
        FLAGS_OFFSET,
        &mut vregs,
    )?;
    emit_fail(
        symbol,
        "ErrTimeout",
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
    // plan-73-C: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the prologue before any non-blocking-mode change, so no flags to restore.
    instructions.push(abi::label(&accept_invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_listener_flags_restore(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "alloc",
        RESTORE_FLAGS_OFFSET,
        LISTENER_FD_OFFSET,
        FLAGS_OFFSET,
        &mut vregs,
    )?;
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// net.localAddress / net.remoteAddress
// ---------------------------------------------------------------------------

pub(crate) fn lower_net_read_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<NetBodyParts, String> {
    const FRAME_SIZE: usize = 96;
    const FD_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const STR_OFFSET: usize = 40;
    // bug-261: the per-call temporary read buffer is capped at this size instead of
    // the caller's `maxBytes`, so a large (or attacker-influenced) `maxBytes` does
    // not pre-commit that much memory for a `read` that delivers far fewer bytes.
    // A single `read()` never returns more than the socket receive buffer (well
    // under 1 MiB by default on every platform), so capping the single-read
    // ceiling here is transparent to the documented "one underlying receive"
    // semantics while removing the pre-allocation amplifier.
    const READ_CHUNK_CAP: &str = "1048576"; // 1 MiB

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let read_size_ok = format!("{symbol}_read_size_ok");
    let peer_closed = format!("{symbol}_peer_closed");
    let read_retry = format!("{symbol}_read_retry");
    let read_fail = format!("{symbol}_read_fail");
    let timeout = format!("{symbol}_timeout");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let build_list = format!("{symbol}_build_list");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&invalid),
        // bug-261: clamp the read size to READ_CHUNK_CAP and store it back, so both
        // the temporary buffer allocation and the read() length use the bounded
        // value (never the caller's raw maxBytes). Keeps alloc proportional to what
        // a single receive can deliver.
        abi::move_immediate(&v11, "Integer", READ_CHUNK_CAP),
        abi::compare_registers(&v10, &v11),
        abi::branch_le(&read_size_ok),
        abi::move_register(&v10, &v11),
        abi::store_u64(&v10, abi::stack_pointer(), MAX_OFFSET),
        abi::label(&read_size_ok),
        // Allocate a temporary read buffer of the (capped) size.
        abi::move_register(abi::return_register(), &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFFSET),
        // read(fd, buf, maxBytes); read_retry reloads the args from the stack so
        // an EINTR retry re-issues the identical call (bug-115).
        abi::label(&read_retry),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MAX_OFFSET),
    ]);
    if platform.family() == PlatformFamily::Windows {
        // recv(s, buf, len, 0). ReadFile does not work on a default (overlapped)
        // Winsock socket; recv is the socket read primitive. Same (fd, buf, len)
        // arg layout, so only the flags word is added. The C `int` return is
        // sign-extended before the 0 (peer-closed) / <0 (error) compares below.
        instructions.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
        platform.emit_external_call(
            net_symbol(platform, NetSymbol::Recv),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::sign_extend_word(
            abi::return_register(),
            abi::return_register(),
        ));
    } else {
        platform.emit_read_file(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_lt(&read_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
    ]);
    if text {
        // Build a String: [u64 len][bytes][nul], validate UTF-8.
        emit_string_result_build(
            symbol,
            BUF_OFFSET,
            N_OFFSET,
            STR_OFFSET,
            &str_copy,
            &str_done,
            &alloc_fail,
            &encoding_error,
            &mut instructions,
            &mut relocations,
        );
        instructions.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR_OFFSET),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&encoding_error),
        ]);
        emit_fail(
            symbol,
            "ErrEncoding",
            &mut instructions,
            &mut relocations,
            &done,
        );
    } else {
        // Build a List OF Byte with N elements.
        instructions.extend([
            abi::label(&build_list),
            abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
            abi::move_immediate(&v11, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v12, &v10, &v11),
            abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), &v12, &v10),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register(&v15, abi::mfb_return(1)), // alloc result -> vreg base (plan-34-B Phase 3)
            abi::move_immediate(&v9, "Byte", &byte_list_block_kind().to_string()),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KIND),
            abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate(&v9, "Byte", "1"),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_COUNT),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_CAPACITY),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate(&v11, &v15, COLLECTION_HEADER_SIZE),
            abi::move_immediate(&v12, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v13, &v10, &v12),
            abi::add_registers(&v14, &v11, &v13),
            // x11 = entry cursor, x14 = data region, copy bytes into data.
            abi::load_u64(&v15, abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate(&v9, "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&entry_done),
        ]);
        // kind 2 has no entry array (plan-57-D): with a zero stride these stores
        // would rewrite one "entry" over the data region `count` times, so they
        // are skipped outright. The payload copy below is NOT guarded — it is the
        // only thing that writes the bytes.
        if byte_list_entry_stride() != 0 {
            instructions.extend([
                abi::move_immediate(&v12, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
                abi::store_u8(&v12, &v11, COLLECTION_ENTRY_OFFSET_FLAGS),
                abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
                abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
                abi::store_u64(&v9, &v11, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
                abi::move_immediate(&v12, "Integer", "1"),
                abi::store_u64(&v12, &v11, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            ]);
        }
        instructions.extend([
            // data[i] = buf[i]
            abi::add_registers(&v12, &v14, &v9),
            abi::load_u8(&v13, &v15, 0),
            abi::store_u8(&v13, &v12, 0),
            abi::add_immediate(&v15, &v15, 1),
        ]);
        if byte_list_entry_stride() != 0 {
            instructions.push(abi::add_immediate(&v11, &v11, COLLECTION_ENTRY_SIZE));
        }
        instructions.extend([
            abi::add_immediate(&v9, &v9, 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
            abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
        ]);
    }
    instructions.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // read_fail: distinguish a read timeout (EAGAIN) from a closed connection.
    instructions.push(abi::label(&read_fail));
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_eq(&timeout),
    ]);
    if platform.family() == PlatformFamily::Windows {
        // SO_RCVTIMEO timeout is WSAETIMEDOUT on Winsock, not EWOULDBLOCK (bug-109).
        instructions.extend([
            abi::compare_immediate(&v9, WSAETIMEDOUT),
            abi::branch_eq(&timeout),
        ]);
    }
    instructions.extend([
        // bug-115: a signal that interrupts the blocking read returns -1/EINTR;
        // re-issue rather than misreporting it as a closed connection.
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&read_retry),
    ]);
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
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
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// net.write / net.writeText
// ---------------------------------------------------------------------------

pub(crate) fn lower_net_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<NetBodyParts, String> {
    const FRAME_SIZE: usize = 96;
    const FD_OFFSET: usize = 8;
    const SRC_OFFSET: usize = 16; // pointer to the next byte to write
    const REMAINING_OFFSET: usize = 24;

    let closed = format!("{symbol}_closed");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_fail = format!("{symbol}_write_fail");
    let timeout = format!("{symbol}_timeout");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
    ]);
    // bug-497 / bug-508: one payload view for every backend — the text form
    // as before, the byte form after a header check (`push_write_payload_view`).
    let bad_payload = format!("{symbol}_bad_payload");
    push_write_payload_view(
        &mut instructions,
        text,
        abi::c_arg(1),
        &v10,
        &v11,
        &v14,
        &v12,
        &v13,
        REMAINING_OFFSET,
        SRC_OFFSET,
        &bad_payload,
    );
    instructions.extend([
        abi::label(&write_loop),
        abi::load_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&write_done),
        // write(fd, src, remaining)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SRC_OFFSET),
        abi::move_register(abi::c_arg(2), &v10),
    ]);
    if platform.family() == PlatformFamily::Windows {
        // send(s, buf, len, 0). WriteFile does not work on a default (overlapped)
        // Winsock socket; send is the socket write primitive. The C `int` return is
        // sign-extended before the <= 0 (error) compare below.
        instructions.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
        platform.emit_external_call(
            net_symbol(platform, NetSymbol::Send),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::sign_extend_word(
            abi::return_register(),
            abi::return_register(),
        ));
    } else {
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_fail),
        abi::load_u64(&v11, abi::stack_pointer(), SRC_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers(&v11, &v11, abi::return_register()),
        abi::subtract_registers(&v10, &v10, abi::return_register()),
        abi::store_u64(&v11, abi::stack_pointer(), SRC_OFFSET),
        abi::store_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_fail),
    ]);
    if write_uses_raw_syscall(platform) {
        // linux-x86_64's `emit_write` is a raw `syscall` that returns `-errno` in
        // the return register and never sets the libc `errno` cell, so reading
        // `__errno_location` here yields a stale value and misreports a write
        // timeout as a closed connection (bug-109). The failing return value is
        // still live (the advance path was branched over): EAGAIN iff
        // `ret == -EAGAIN`, i.e. `ret + EAGAIN == 0` — mirroring the fs/io raw
        // branch (bug-62).
        let eagain = platform
            .socket_would_block_code()
            .parse::<usize>()
            .expect("eagain is numeric");
        let eintr = EINTR_ERRNO.parse::<usize>().expect("eintr is numeric");
        instructions.extend([
            abi::add_immediate(&v9, abi::return_register(), eagain),
            abi::compare_immediate(&v9, "0"),
            abi::branch_eq(&timeout),
            // bug-115: EINTR (raw `-errno`) re-issues the write from write_loop,
            // which reloads the unchanged cursor/remaining (no bytes moved).
            abi::add_immediate(&v9, abi::return_register(), eintr),
            abi::compare_immediate(&v9, "0"),
            abi::branch_eq(&write_loop),
        ]);
    } else {
        // Every other backend routes `write` through libc: a `-1` return with the
        // real code behind the `errno` accessor.
        platform.emit_errno(
            symbol,
            (&v9).into(),
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&v9, platform.socket_would_block_code()),
            abi::branch_eq(&timeout),
        ]);
        if platform.family() == PlatformFamily::Windows {
            // A blocking send that hits SO_SNDTIMEO returns WSAETIMEDOUT (10060) on
            // Winsock, not WSAEWOULDBLOCK — map it to the same write timeout (bug-109).
            instructions.extend([
                abi::compare_immediate(&v9, WSAETIMEDOUT),
                abi::branch_eq(&timeout),
            ]);
        }
        instructions.extend([
            // bug-115: a signal that interrupts the blocking write returns
            // -1/EINTR; re-issue from write_loop rather than reporting a closed
            // connection.
            abi::compare_immediate(&v9, EINTR_ERRNO),
            abi::branch_eq(&write_loop),
        ]);
    }
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
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
    if !text {
        // bug-497: the byte form was handed a block whose header is not a
        // `List OF Byte`'s — refuse rather than read a length out of its bytes.
        instructions.push(abi::label(&bad_payload));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// net.lookup
// ---------------------------------------------------------------------------
