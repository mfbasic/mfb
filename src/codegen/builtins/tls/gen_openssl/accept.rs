//! plan-176-T: the OpenSSL `tls::accept` lowering (`lower_tls_accept_openssl`) and its
//! monotonic clock. The shared imports live in `builtins/tls/gen_openssl/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// tls.accept
// ---------------------------------------------------------------------------

/// bug-739: `clock_gettime(CLOCK_MONOTONIC, sp + ts_off)`, then leave the
/// reading in whole milliseconds (`tv_sec * 1000 + tv_nsec / 1_000_000`) in the
/// first word of the same scratch, where the caller reloads it. Every value
/// is re-read from the stack, so nothing is held in a register across the call.
fn emit_monotonic_ms(
    platform: &dyn CodegenPlatform,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    ts_off: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let sec = vregs.next();
    let nsec = vregs.next();
    let k = vregs.next();
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"), // CLOCK_MONOTONIC
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ts_off),
    ]);
    platform.emit_external_call(
        "clock_gettime",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(&sec, abi::stack_pointer(), ts_off),
        abi::load_u64(&nsec, abi::stack_pointer(), ts_off + 8),
        abi::move_immediate(&k, "Integer", "1000"),
        abi::multiply_registers(&sec, &sec, &k),
        abi::move_immediate(&k, "Integer", "1000000"),
        abi::unsigned_divide_registers(&nsec, &nsec, &k),
        abi::add_registers(&sec, &sec, &nsec),
        abi::store_u64(&sec, abi::stack_pointer(), ts_off),
    ]);
    Ok(())
}

pub(crate) fn lower_tls_accept_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 128;
    const FD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    const CONNFD_OFFSET: usize = 24;
    const CTX_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;
    const SSL_OFFSET: usize = 56;
    const POLLFD_OFFSET: usize = 64; // pollfd { fd; events; revents }
    const TIMEVAL_OFFSET: usize = 72; // timespec scratch for clock_gettime (bug-739)
    const HSTOFLAG: usize = 88; // plan-73-D: 1 if the handshake timed out
    const DEADLINE_OFFSET: usize = 96; // bug-739: handshake deadline, monotonic ms
    const FLAGS_OFFSET: usize = 104; // bug-739: the connfd's F_GETFL flags
    const ACCEPTFN_OFFSET: usize = 112; // bug-739: SSL_accept
    const GETERRFN_OFFSET: usize = 120; // bug-739: SSL_get_error

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let no_timeout = format!("{symbol}_no_timeout");
    let accept_fail = format!("{symbol}_accept_fail");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let accept_invalid = format!("{symbol}_accept_invalid");
    let accept_ts_store = format!("{symbol}_accept_ts_clamped");
    let ssl_fail = format!("{symbol}_ssl_fail");
    let tls_fail_conn = format!("{symbol}_tls_fail_conn");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    // x0 = listener record { fd@0, closed@8, ctx@16 }; x1 = timeoutMs.
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOFLAG),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
    ]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        // plan-73-D: the unbounded sentinel => a blocking accept (omit = block); `0`
        // => poll(POLLIN, 0), one immediate attempt (`ErrTimeout` if none pending);
        // `> 0` => poll(POLLIN, timeoutMs); a negative (non-sentinel) => invalid.
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&no_timeout),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&accept_invalid),
        // Clamp `> 0` to INT_MAX and store it back — poll() takes a C `int`, so a
        // value with bit 31 set would be read as a block-forever timeout (bug-239);
        // net clamps identically. The sentinel skips this (branch_eq above).
        abi::move_immediate(&v10, "Integer", "2147483647"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&accept_ts_store),
        abi::move_register(&v9, &v10),
        abi::label(&accept_ts_store),
        abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"), // POLLIN
        abi::store_u8(&v10, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `poll` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::branch_eq(&accept_timeout),
        abi::label(&no_timeout),
        // accept4(fd, NULL, NULL, SOCK_CLOEXEC) (bug-499)
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
        // bug-102.3: narrow the C int `accept` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONNFD_OFFSET),
    ]);
    // --- Server-side handshake on the accepted fd ---
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &tls_fail_conn,
    )?;
    // ssl = SSL_new(listener.ctx)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_new",
        FNPTR_OFFSET,
        &tls_fail_conn,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail_conn),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), SSL_OFFSET),
    ]);
    // SSL_set_fd(ssl, connfd)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set_fd",
        FNPTR_OFFSET,
        &ssl_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CONNFD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ssl_fail),
    ]);
    // Bound the server handshake by timeoutMs. `timeoutMs` once bounded only the
    // connection-wait poll above, so a client that completed the TCP handshake
    // and then stalled mid-TLS wedged SSL_accept, and the single-threaded accept
    // loop with it, forever (bug-202). The fix for that was SO_RCVTIMEO/SO_SNDTIMEO,
    // which bound each read and write, not the handshake: a peer that trickled
    // its ClientHello a byte at a time held SSL_accept for as long as it liked
    // (bug-739). So a bounded handshake now runs non-blocking against ONE
    // CLOCK_MONOTONIC deadline taken here, when the connection has been taken:
    // SSL_accept, and on WANT_READ/WANT_WRITE poll the fd for the time left.
    // Sentinel => a plain blocking SSL_accept (omit = block); `0` => one
    // non-blocking attempt, then ErrTimeout; `> 0` => deadline = now + timeoutMs.
    // The fd's blocking mode is restored on success so the returned socket's
    // reads and writes stay unbounded.
    let hs_blocking = format!("{symbol}_hs_blocking");
    let hs_loop = format!("{symbol}_hs_loop");
    let hs_want_write = format!("{symbol}_hs_want_write");
    let hs_poll = format!("{symbol}_hs_poll");
    let hs_poll_err = format!("{symbol}_hs_poll_err");
    let hs_expired = format!("{symbol}_hs_expired");
    let hs_nb_done = format!("{symbol}_hs_nb_done");
    let asc_ok = format!("{symbol}_asc_ok");
    instructions.extend([
        abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v14, &v15),
        abi::branch_eq(&hs_blocking),
    ]);
    emit_monotonic_ms(
        platform,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
        TIMEVAL_OFFSET,
        &mut vregs,
    )?;
    instructions.extend([
        abi::load_u64(&v11, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::load_u64(&v12, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::add_registers(&v11, &v11, &v12),
        abi::store_u64(&v11, abi::stack_pointer(), DEADLINE_OFFSET),
        // flags = fcntl(connfd, F_GETFL, 0)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "3"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
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
    platform.emit_set_nonblocking(
        CONNFD_OFFSET,
        FLAGS_OFFSET,
        vregs.next().into(),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    for (name, slot) in [
        ("SSL_accept", ACCEPTFN_OFFSET),
        ("SSL_get_error", GETERRFN_OFFSET),
    ] {
        emit_dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            HANDLE_OFFSET,
            name,
            FNPTR_OFFSET,
            &ssl_fail,
        )?;
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), slot),
        ]);
    }
    instructions.extend([
        // r = SSL_accept(ssl): 1 = done.
        abi::label(&hs_loop),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), ACCEPTFN_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_eq(&hs_nb_done),
        // e = SSL_get_error(ssl, r): WANT_READ (2) => POLLIN, WANT_WRITE (3) =>
        // POLLOUT, anything else is a TLS failure.
        abi::move_register(abi::c_arg(1), abi::c_return(0)),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), GETERRFN_OFFSET),
        abi::branch_link_register(&v9),
        // A native call's result is in c_return(0): on x86-64 that is rax, not
        // the MFB return register (`.ai/arch-abi.md`).
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::move_register(&v10, abi::c_return(0)),
        abi::move_immediate(&v13, "Integer", "1"), // POLLIN
        abi::compare_immediate(&v10, "2"),
        abi::branch_eq(&hs_poll),
        abi::compare_immediate(&v10, "3"),
        abi::branch_ne(&ssl_fail),
        abi::label(&hs_want_write),
        abi::move_immediate(&v13, "Integer", "4"), // POLLOUT
        abi::label(&hs_poll),
        abi::load_u64(&v9, abi::stack_pointer(), CONNFD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::store_u8(&v13, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
    ]);
    emit_monotonic_ms(
        platform,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
        TIMEVAL_OFFSET,
        &mut vregs,
    )?;
    instructions.extend([
        // left = deadline - now; nothing left => the handshake timed out.
        abi::load_u64(&v11, abi::stack_pointer(), DEADLINE_OFFSET),
        abi::load_u64(&v12, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::subtract_registers(&v11, &v11, &v12),
        abi::compare_immediate(&v11, "0"),
        abi::branch_le(&hs_expired),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), &v11),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `poll` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&hs_loop),
        abi::branch_lt(&hs_poll_err),
        // poll timed out: the deadline passed while the peer said nothing.
        abi::label(&hs_expired),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::branch(&ssl_fail),
        abi::label(&hs_poll_err),
    ]);
    // A signal interrupted the poll (EINTR = 4 on every Linux target): go round
    // again; the deadline check bounds the retries. Anything else is a failure.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, "4"),
        abi::branch_eq(&hs_loop),
        abi::branch(&ssl_fail),
        // Handshake done: restore the fd's blocking mode, fcntl(connfd, F_SETFL, flags).
        abi::label(&hs_nb_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "4"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), FLAGS_OFFSET),
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&ssl_fail),
        abi::branch(&asc_ok),
        // Omitted timeoutMs: one blocking SSL_accept, unbounded by design.
        abi::label(&hs_blocking),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_accept",
        FNPTR_OFFSET,
        &ssl_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ssl_fail),
        abi::label(&asc_ok),
    ]);
    // Build the Socket record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the TLS tail { ssl, ctx = 0 } (plan-80) — the zero ctx slot marks a
    // non-owned (listener-owned) server context, which the close helper must not
    // free (plan-06-tls-server.md §6.4).
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_OPENSSL),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), CONNFD_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        // bug-564: no read has reported the peer's close yet.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_PEER_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_SSL),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    // ssl_fail: free the session, then close the accepted fd.
    instructions.push(abi::label(&ssl_fail));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &tls_fail_conn,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&tls_fail_conn));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        CONNFD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // plan-73-D: a handshake recv that hit the SO_*TIMEO is a timeout; else TLS fail.
    let accept_hs_timeout = format!("{symbol}_accept_hs_timeout");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&accept_hs_timeout),
    ]);
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&accept_hs_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&accept_fail));
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&accept_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the up-front selector before any accept/alloc, so nothing to clean up.
    instructions.push(abi::label(&accept_invalid));
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
    instructions.push(abi::label(&closed_moved));
    emit_fail(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    // The record alloc is the final step; the SSL session and the accepted fd
    // are both live, so free/close them before failing — a record-alloc OOM
    // otherwise leaked the SSL session and the accepted socket fd (bug-55).
    // Both are always set on the only path here. SSL_free's dlsym failure (only
    // if libssl vanished) routes to tls_fail_conn, which still closes the fd.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &tls_fail_conn,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD_OFFSET),
    ]);
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
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
