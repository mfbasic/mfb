//! plan-176-T: the Network.framework `tls::accept` lowering (`lower_tls_accept_macos`) and its
//! deadline. The shared imports live in `builtins/tls/gen_macos/server/mod.rs`.

use super::*;

/// bug-736: store `tls::accept`'s deadline for one phase at `sp + deadline_off`,
/// from the clamped `timeoutMs` at `sp + timeout_off`. The unbounded sentinel is
/// `DISPATCH_TIME_FOREVER` (omitted = block), `0` is `DISPATCH_TIME_NOW` (one
/// immediate attempt, `ErrTimeout` if not already done), and a positive value is
/// `dispatch_time(DISPATCH_TIME_NOW, ms * 1e6)` read at the moment this runs.
/// `accept` calls it once before the connection wait and again when it pops a
/// connection, so each phase is bounded by the full `timeoutMs` and the time the
/// connection wait used is never taken from the handshake. Linux and Windows
/// get the same property by reloading the raw value into the accepted socket's
/// `SO_RCVTIMEO`/`SO_SNDTIMEO`, though there it bounds each handshake read or
/// write rather than the whole handshake. `tag` keeps the two call sites'
/// labels apart; `fnptr_off` is clobbered.
#[allow(clippy::too_many_arguments)]
fn emit_accept_deadline(
    ctx: &mut EmitCtx,
    handle_off: usize,
    timeout_off: usize,
    fnptr_off: usize,
    deadline_off: usize,
    tag: &str,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
    let v10 = vregs.next();
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let wait_now = format!("{symbol}_{tag}_wait_now");
    let wait_forever = format!("{symbol}_{tag}_wait_forever");
    let ready = format!("{symbol}_{tag}_deadline_ready");

    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), timeout_off),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&wait_forever),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&wait_now),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        "dispatch_time",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), timeout_off),
        abi::move_immediate(&v10, "Integer", "1000000"),
        abi::multiply_registers(abi::c_arg(1), abi::c_arg(1), &v10), // ms -> ns
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), deadline_off),
        abi::branch(&ready),
        abi::label(&wait_now),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), deadline_off),
        abi::branch(&ready),
        abi::label(&wait_forever),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9), // DISPATCH_TIME_FOREVER
        abi::store_u64(&v9, abi::stack_pointer(), deadline_off),
        abi::label(&ready),
    ]);
    Ok(())
}

pub(crate) fn lower_tls_accept_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    const FRAME_SIZE: usize = 208;
    const REC: usize = 8;
    const TIMEOUT: usize = 16;
    const NWH: usize = 24;
    const FNPTR: usize = 32;
    const LCTX: usize = 40;
    const QUEUE: usize = 48;
    const DEADLINE: usize = 56;
    const WAITFN: usize = 64;
    const CONN: usize = 72;
    const CCTX: usize = 80;
    const SBLOCK: usize = 96; // 96..136: per-connection state block literal

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let wait_loop = format!("{symbol}_wait");
    let pop = format!("{symbol}_pop");
    let listener_dead = format!("{symbol}_listener_dead");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let accept_invalid = format!("{symbol}_accept_invalid");
    let hs_loop = format!("{symbol}_hs_wait");
    let hs_timeout = format!("{symbol}_hs_timeout");
    let hs_timeout_drain = format!("{symbol}_hs_timeout_drain");
    let conn_fail = format!("{symbol}_conn_fail");
    let conn_fail_drain = format!("{symbol}_conn_fail_drain");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    // x0 = listener record { closed@0, listener@8, queue@16, lctx@24 };
    // x1 = timeoutMs.
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
    ]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    {
        // plan-73-D: reject a negative (non-sentinel) `timeoutMs` up front (nothing
        // allocated yet). The omitted overload pads the unbounded sentinel (→ FOREVER
        // at the deadline block); `0`/`> 0` pass through. `x0` still holds the record
        // here, so preserve it — use a scratch that the following block reloads.
        let ts_ok = format!("{symbol}_ts_ok");
        let ts_store = format!("{symbol}_ts_clamped");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
            abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&ts_ok),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&accept_invalid),
            // Clamp `> 0` to INT_MAX and store it back so `ms * 1e6` cannot overflow
            // and a huge timeout is bounded identically across backends. The reject
            // block uses only v9/v10, so x0 (the record) is still preserved below.
            abi::move_immediate(&v10, "Integer", "2147483647"),
            abi::compare_registers(&v9, &v10),
            abi::branch_le(&ts_store),
            abi::move_register(&v9, &v10),
            abi::label(&ts_store),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT),
            abi::label(&ts_ok),
        ]);
    }
    ins.extend([
        // x0 still holds the listener record (the reject block above used only v9/v10).
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), LCTX),
        abi::load_u64(&v9, abi::return_register(), REC_QUEUE),
        abi::store_u64(&v9, abi::stack_pointer(), QUEUE),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        &load_fail,
    )?;
    // plan-73-D / bug-736. Deadline for the CONNECTION wait. The same timeout is
    // applied again, from a fresh `dispatch_time(NOW, ..)`, once a connection is
    // popped (see `pop` below), so the handshake never inherits what the wait
    // spent. Linux and Windows reload the raw value for their handshake
    // `SO_*TIMEO`, which has the same effect.
    emit_accept_deadline(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        TIMEOUT,
        FNPTR,
        DEADLINE,
        "conn",
        &load_fail,
        &mut vregs,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), WAITFN),
        // Wait for a queued connection (the ring is checked first so
        // connections that arrived before this accept are drained even when
        // their semaphore counts were consumed by earlier state wakeups).
        abi::label(&wait_loop),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::load_u64(&v10, &v9, LCTX_HEAD),
        abi::load_u64(&v11, &v9, LCTX_TAIL),
        abi::compare_registers(&v10, &v11),
        abi::branch_ne(&pop),
        // Listener failed/cancelled while we wait?
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, NW_LISTENER_FAILED),
        abi::branch_ge(&listener_dead),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DEADLINE),
        abi::load_u64(&v10, abi::stack_pointer(), WAITFN),
        abi::branch_link_register(&v10),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&accept_timeout),
        abi::branch(&wait_loop),
        // Pop the oldest queued connection.
        abi::label(&pop),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::load_u64(&v11, &v9, LCTX_TAIL),
        abi::move_immediate(&v12, "Integer", "15"),
        abi::and_registers(&v12, &v11, &v12),
        abi::shift_left_immediate(&v12, &v12, 3),
        abi::add_immediate(&v13, &v9, LCTX_RING),
        abi::add_registers(&v13, &v13, &v12),
        abi::load_u64(&v14, &v13, 0),
        abi::store_u64(&v14, abi::stack_pointer(), CONN),
        abi::add_immediate(&v11, &v11, 1),
        abi::store_u64(&v11, &v9, LCTX_TAIL),
    ]);
    // bug-736: the handshake gets its own deadline, computed now that the
    // connection is out of the backlog. Reusing the connection-wait deadline gave
    // a connection that arrived late in the window only the remainder, so the
    // server cancelled a healthy handshake with `ErrTimeout`.
    emit_accept_deadline(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        TIMEOUT,
        FNPTR,
        DEADLINE,
        "hs",
        &load_fail,
        &mut vregs,
    )?;
    // Per-connection block context { sem, signal, state, content, error }, on the
    // C heap (bug-623 D, see `CTX_SIZE`).
    emit_ctx_calloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        CTX_SIZE,
        &alloc_fail,
    )?;
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        CCTX,
    ));
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::store_u64(abi::return_register(), &v9, CTX_SEM),
        abi::store_u64(abi::ZERO, &v9, CTX_STATE),
        abi::store_u64(abi::ZERO, &v9, CTX_CONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_ERROR),
    ]);
    // plan-76-B Phase 4: dedicated poll-receive semaphore + zeroed pending slots on
    // the server-accepted connection ctx (tls::poll works on accepted sockets too).
    // FNPTR still holds dispatch_semaphore_create here.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::store_u64(abi::return_register(), &v9, CTX_PSEM),
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_PERROR),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_BUF),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
        abi::store_u64(abi::ZERO, &v9, CTX_ARMED),
        // plan-110-D: no read/write deadline until one is installed. The
        // sentinel is what makes the waits below stay FOREVER on a socket
        // whose owner never called a timeout setter.
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::store_u64(&v10, &v9, CTX_RTO),
        abi::store_u64(&v10, &v9, CTX_WTO),
        abi::store_u64(abi::ZERO, &v9, CTX_WARMED),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::store_u64(&v10, &v9, CTX_SIGNAL),
    ]);
    // bug-483: ctx->edomfn = &nw_error_get_error_domain, and edom = 0 ("no error
    // classified yet"). Only the send trampoline may touch an `nw_error` — it is
    // released when the completion block returns — so the pointer is parked here
    // for it.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_error_get_error_domain",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::store_u64(&v10, &v9, CTX_EDOMFN),
        abi::store_u64(abi::ZERO, &v9, CTX_EDOM),
    ]);
    // nw_connection_set_queue(conn, queue) — the listener's serial queue.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_connection_set_queue",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // Per-connection state handler, then start (runs the server handshake).
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        STATE_INVOKE,
        CCTX,
        SBLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_connection_set_state_changed_handler",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), SBLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_connection_start",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Wait for the connection to reach ready (handshake complete).
        abi::label(&hs_loop),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DEADLINE),
        abi::load_u64(&v10, abi::stack_pointer(), WAITFN),
        abi::branch_link_register(&v10),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&hs_timeout),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, NW_STATE_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate(&v10, "2"), // preparing
        abi::branch_eq(&hs_loop),
        abi::compare_immediate(&v10, "0"), // invalid
        abi::branch_eq(&hs_loop),
        abi::branch(&conn_fail), // waiting/failed/cancelled
        abi::label(&ready),
    ]);
    // Build the Socket record { closed=0, conn, queue=0, cctx } — the queue
    // slot is 0 (not the listener's shared serial queue) so the shared close
    // helper releases the connection and ctx semaphore this socket owns but not
    // the listener-owned queue, which closeListener releases (bug-55). read/
    // write/close otherwise work identically to a client socket.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_MACOS),
        abi::store_u64(&v9, abi::mfb_return(1), REC_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_STATE),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), CONN),
        abi::store_u64(&v9, abi::mfb_return(1), REC_CONN),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), CCTX),
        abi::store_u64(&v9, abi::mfb_return(1), REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // conn_fail / hs_timeout: cancel the accepted connection, then drop the
    // reference `accept` owns. The new-connection trampoline `nw_retain`s the
    // connection into the ring and the successful path releases it at close;
    // cancelling alone tears down network activity but keeps the +1, so before
    // bug-317 every handshake failure leaked one nw_connection — a remotely
    // triggerable, unbounded server-side leak for a server looping on accept.
    ins.push(abi::label(&conn_fail));
    emit_cancel_and_release_conn(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        CONN,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    emit_cancel_drain(&mut ins, CCTX, WAITFN, &conn_fail_drain, "5", &mut vregs);
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&hs_timeout));
    emit_cancel_and_release_conn(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        CONN,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    emit_cancel_drain(&mut ins, CCTX, WAITFN, &hs_timeout_drain, "5", &mut vregs);
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&listener_dead));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed_moved));
    emit_fail(symbol, "ErrResourceMoved", &mut ins, &mut rel, &done);
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument (rejected
    // up front, nothing allocated).
    ins.push(abi::label(&accept_invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
