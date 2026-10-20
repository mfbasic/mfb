//! plan-176-T: the Network.framework `tls::closeListener` lowering
//! (`lower_tls_close_listener_macos`). The shared imports live in
//! `builtins/tls/gen_macos/server/mod.rs`.

use super::*;

pub(crate) fn lower_tls_close_listener_macos(
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
    const FRAME_SIZE: usize = 96;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    const LCTX: usize = 32;
    const CONN: usize = 40;
    const SETQFN: usize = 48;
    const CANCELFN: usize = 56;
    const RELEASEFN: usize = 64;
    const WAITFN: usize = 72;

    let already = format!("{symbol}_already");
    let already_moved = format!("{symbol}_already_moved");
    let load_fail = format!("{symbol}_load_fail");
    let drain_loop = format!("{symbol}_drain");
    let drained = format!("{symbol}_drained");
    // bug-412: the listener-cancel drain (distinct from `drain_loop`, which
    // rejects still-queued connections). Blocks on the listener ctx semaphore
    // until the async `nw_listener_cancel` reaches `cancelled`.
    let lcancel_drain = format!("{symbol}_lcancel_drain");
    let skip_lctx_free = format!("{symbol}_skip_lctx_free");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        // Idempotent: a closed handle returns OK.
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
    ]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &already,
        &already_moved,
    ));
    ins.extend([
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), LCTX),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        &load_fail,
    )?;
    for (name, off) in [
        ("nw_connection_set_queue", SETQFN),
        ("nw_connection_cancel", CANCELFN),
        ("nw_release", RELEASEFN),
    ] {
        dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            HANDLE,
            name,
            FNPTR,
            &load_fail,
        )?;
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
            abi::store_u64(&v9, abi::stack_pointer(), off),
        ]);
    }
    // Reject every still-queued (retained, never-started) connection: give it
    // the listener's queue, cancel it, drop our retain.
    ins.extend([
        abi::label(&drain_loop),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::load_u64(&v10, &v9, LCTX_HEAD),
        abi::load_u64(&v11, &v9, LCTX_TAIL),
        abi::compare_registers(&v10, &v11),
        abi::branch_eq(&drained),
        abi::move_immediate(&v12, "Integer", "15"),
        abi::and_registers(&v12, &v11, &v12),
        abi::shift_left_immediate(&v12, &v12, 3),
        abi::add_immediate(&v13, &v9, LCTX_RING),
        abi::add_registers(&v13, &v13, &v12),
        abi::load_u64(&v14, &v13, 0),
        abi::store_u64(&v14, abi::stack_pointer(), CONN),
        abi::add_immediate(&v11, &v11, 1),
        abi::store_u64(&v11, &v9, LCTX_TAIL),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::c_arg(1), &v9, REC_QUEUE),
        abi::load_u64(&v10, abi::stack_pointer(), SETQFN),
        abi::branch_link_register(&v10),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(&v10, abi::stack_pointer(), CANCELFN),
        abi::branch_link_register(&v10),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(&v10, abi::stack_pointer(), RELEASEFN),
        abi::branch_link_register(&v10),
        abi::branch(&drain_loop),
        abi::label(&drained),
    ]);
    // nw_listener_cancel(listener)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_listener_cancel",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_CONN),
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v10),
    ]);
    // bug-412: `nw_listener_cancel` is asynchronous; the listener state handler
    // (STATE_INVOKE over the arena-allocated LCTX) still fires the `cancelled`
    // transition and signals LCTX->sem. Drain to that terminal state (listener
    // state 4) before returning, so a process exit here cannot leave a queued
    // handler to dereference the freed LCTX. Done while the listener, its queue,
    // and the ctx are all still retained (before the releases below), mirroring
    // the connect-path drain (bug-380). WAITFN gets `dispatch_semaphore_wait`.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), WAITFN),
    ]);
    emit_cancel_drain(&mut ins, LCTX, WAITFN, &lcancel_drain, "4", &mut vregs);
    ins.extend([
        // Release the listener, its serial queue, and the listener-ctx
        // semaphore this handle owns; cancelling alone leaks them (bug-55). The
        // arena-allocated lctx block is reclaimed with the arena. RELEASEFN
        // already holds nw_release (resolved in the drain loop above).
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_CONN),
        abi::load_u64(&v10, abi::stack_pointer(), RELEASEFN),
        abi::branch_link_register(&v10),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // NB: the listener ctx semaphore is intentionally NOT released here, for
        // the same reason as the connection close: nw_listener_cancel is async
        // and the listener state handler still signals ctx->sem on the cancelled
        // transition. It is reclaimed with the arena-allocated lctx block.
        //
        // bug-623: the lctx block itself goes back to the C heap now. The cancel
        // drain above reached the terminal state, so neither the state handler
        // nor the new-connection handler runs again, and `tls::accept` checks
        // REC_CLOSED before it loads REC_CTX. A transferred listener's lctx,
        // allocated by another thread's arena, is freed here too: a free touches
        // only the freeing thread's bins (bug-623 D, see `CTX_SIZE`).
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::move_register(abi::c_arg(0), &v9),
    ]);
    emit_ctx_free(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut ins,
        relocations: &mut rel,
    })?;
    // bug-984: the bound-host C string `tls::listen` parked at `REC_LHOST` is the
    // listener's own arena block, and this close is where its last reader goes:
    // `tls::localAddress(listener)` refuses a closed record before it loads the
    // word. Freed by `strlen + 1`, the count both of its allocators used — the
    // marshalled host is `byteLength + 1` and the thread-transfer copy measures
    // the same way. (A host with an embedded NUL measures short, which returns
    // the block to a smaller bin: wasted bytes, never a wrong free.) The word is
    // nulled, and a second close takes `already` above without reaching here.
    let host_scan = format!("{symbol}_lhost_scan");
    let host_measured = format!("{symbol}_lhost_measured");
    let host_absent = format!("{symbol}_lhost_absent");
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::store_u64(abi::ZERO, &v9, REC_CTX),
        abi::load_u64(v10, v9, REC_LHOST),
        abi::compare_immediate(v10, "0"),
        abi::branch_eq(&host_absent),
        abi::move_register(v11, v10),
        abi::move_immediate(v12, "Integer", "1"),
        abi::label(&host_scan),
        abi::load_u8(v13, v11, 0),
        abi::compare_immediate(v13, "0"),
        abi::branch_eq(&host_measured),
        abi::add_immediate(v11, v11, 1),
        abi::add_immediate(v12, v12, 1),
        abi::branch(&host_scan),
        abi::label(&host_measured),
        abi::move_register(abi::c_arg(0), v10),
        abi::move_register(abi::c_arg(1), v12),
    ]);
    emit_arena_free(symbol, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), REC),
        abi::store_u64(abi::ZERO, v9, REC_LHOST),
        abi::label(&host_absent),
    ]);
    ins.extend([
        abi::label(&skip_lctx_free),
        // Mark closed.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, REC_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&already));
    // bug-525: an already-closed handle is refused, not reported as success.
    // `mfb spec language resource-management` §15: "an already-closed record is
    // flagged, and a second close is a defined no-op reported as
    // `ErrResourceClosed` rather than an operation on a dead handle" — the same
    // answer `fs`, `tcp` and `udp` have always given. A drop-time re-close still
    // costs nothing: lexical cleanup discards a close failure (§15), so the
    // "close once, or let the scope do it" idiom is unaffected.
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&already_moved));
    emit_fail(symbol, "ErrResourceMoved", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
