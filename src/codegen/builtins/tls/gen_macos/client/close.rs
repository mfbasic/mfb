//! plan-176-T: the Network.framework `tls::close` lowering (`lower_tls_close_macos`). The
//! shared imports live in `builtins/tls/gen_macos/client/mod.rs`.

use super::*;

pub(crate) fn lower_tls_close_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 48;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    // bug-564: the cancel drain needs a resolved `dispatch_semaphore_wait` and
    // the ctx pointer in stack slots of their own.
    const WAITFN: usize = 32;
    const CTX: usize = 40;
    let cancel_drain = format!("{symbol}_cancel_drain");
    let already = format!("{symbol}_already");
    let already_moved = format!("{symbol}_already_moved");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");
    // plan-76-B Phase 4: free/release the poll-receive pending state on close.
    let skip_pfree = format!("{symbol}_skip_pfree");
    let skip_prelease = format!("{symbol}_skip_prelease");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
    ]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &already,
        &already_moved,
    ));
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
    // nw_connection_cancel(conn)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_CONN),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // bug-564: `nw_connection_cancel` is ASYNCHRONOUS. The connection's
    // state-changed handler (STATE_INVOKE) runs on the `mfb.tls` queue and
    // dereferences the arena-allocated ctx on *every* invocation, including the
    // final `cancelled` transition this cancel produces. Returning before that
    // transition fires leaves the handler queued — and if the program then
    // exits, `_mfb_shutdown` calls `arena_destroy`, which **munmaps** the block
    // holding the ctx. The handler runs afterwards, during libc `exit`, and
    // faults on an unmapped page: EXC_BAD_ACCESS / KERN_INVALID_ADDRESS, signal
    // 11, *after* the program has printed all of its output.
    //
    // The comment on the releases below used to be the whole story — "the
    // arena-allocated ctx block is reclaimed with the arena" — and it is only
    // true while the arena still exists. `skip_entry_arena_destroy`
    // (`codegen::engine::builder`) skips the teardown free only for a program
    // embedding a `thread.` runtime call, on the premise that nothing else can
    // outlive `main`; a Network.framework dispatch queue with a pending handler
    // is exactly such a thing, and that gate cannot see it.
    //
    // `cancelled` (connection state 5) is terminal, so waiting for it
    // guarantees no handler runs afterwards. Draining HERE — before the
    // releases below — keeps the connection, its queue and the semaphore all
    // retained for the handler that is about to run. This is the same drain
    // `closeListener` already does for the listener (bug-412) and the
    // connect-failure exit does for a half-open connection (bug-380); ordinary
    // `tls::close` was the one cancel site of the five that did not.
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
        // The ctx pointer is never NULL for an open (non-closed) socket.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(&v10, &v9, REC_CTX),
        abi::store_u64(&v10, abi::stack_pointer(), CTX),
    ]);
    emit_cancel_drain(&mut ins, CTX, WAITFN, &cancel_drain, "5", &mut vregs);
    // Release the connection, its dispatch queue, and the ctx semaphore that
    // this socket owns; cancelling alone leaves them all leaked on every
    // connect+close (bug-55). The arena-allocated ctx block is reclaimed with
    // the arena. Slots are never NULL for an open (non-closed) socket.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_CONN),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    let skip_queue = format!("{symbol}_skip_queue_release");
    let skip_ctx_free = format!("{symbol}_skip_ctx_free");
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
        // Release the queue only if this socket owns it. A client socket stores
        // its own per-connection queue here; an accepted socket stores 0 because
        // it shares the listener's serial queue (released by closeListener), and
        // releasing that shared queue per accepted-close would over-release it.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_QUEUE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_queue),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::label(&skip_queue),
        // NB: ctx->sem is intentionally NOT released here. nw_connection_cancel
        // is asynchronous; the connection's state-changed handler still fires a
        // "cancelled" transition afterwards and does
        // dispatch_semaphore_signal(ctx->sem) — releasing the semaphore now
        // would make that a use-after-free. The single per-connection semaphore
        // is reclaimed with the ctx block (bug-55: the leaks
        // that scale — one per readText/write — are fixed in emit_fresh_sem).
        // plan-76-B Phase 4: free any buffered poll plaintext (arena) and release an
        // unconsumed poll receive's retained content. CTX_PSEM, like ctx->sem, is
        // NOT released (async cancel would race it); it is reclaimed with the arena
        // ctx block. FNPTR still holds dispatch_release from the queue release above.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(&v10, &v9, REC_CTX),
        abi::load_u64(abi::return_register(), &v10, CTX_PEND_BUF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_pfree),
        abi::load_u64(abi::c_arg(1), &v10, CTX_PEND_LEN),
    ]);
    emit_arena_free(symbol, &mut ins, &mut rel);
    ins.extend([
        abi::label(&skip_pfree),
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(&v10, &v9, REC_CTX),
        abi::load_u64(abi::return_register(), &v10, CTX_PCONTENT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_prelease),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::label(&skip_prelease),
        // bug-623: return the ctx block to the C heap. Safe here and only here:
        // the cancel drain above waited for the terminal `cancelled` state, so the
        // state handler cannot run again, and every other tls:: operation checks
        // REC_CLOSED before it loads REC_CTX. Kept (as before) when a poll
        // receive (CTX_ARMED) or a timed-out send (CTX_WARMED) is still
        // outstanding — its completion block holds this address and may still
        // write through it. A transferred socket's ctx, allocated by another
        // thread's arena, is freed here too: a free touches only the freeing
        // thread's bins (bug-623 D, see `CTX_SIZE`). The slot is zeroed after the
        // free.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(&v10, &v9, REC_CTX),
        abi::load_u64(&v9, &v10, CTX_ARMED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&skip_ctx_free),
        abi::load_u64(&v9, &v10, CTX_WARMED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&skip_ctx_free),
        abi::move_register(abi::c_arg(0), &v10),
    ]);
    emit_ctx_free(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut ins,
        relocations: &mut rel,
    })?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::store_u64(abi::ZERO, &v9, REC_CTX),
        abi::label(&skip_ctx_free),
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
