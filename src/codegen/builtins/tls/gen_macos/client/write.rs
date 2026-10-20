//! plan-176-T: the Network.framework `tls::write` lowering (`lower_tls_write_macos`). The
//! shared imports live in `builtins/tls/gen_macos/client/mod.rs`.

use super::*;

pub(crate) fn lower_tls_write_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v14 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 160;
    const REC: usize = 8;
    const CONN: usize = 16;
    const CTX: usize = 24;
    const HANDLE: usize = 32;
    const FNPTR: usize = 40;
    const CONTENT: usize = 48;
    const DATA: usize = 56;
    const DLEN: usize = 64;
    const CTXDEF: usize = 72;
    const BLOCK: usize = 80; // 80..120
    const DEADLINE: usize = 120; // plan-110-D: computed dispatch_time for the send wait

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let write_fail = format!("{symbol}_write_fail");
    let load_fail = format!("{symbol}_load_fail");
    let empty = format!("{symbol}_empty");
    let done = format!("{symbol}_done");
    // plan-110-D: the write deadline installed by `tls::setWriteTimeout`.
    let write_timeout = format!("{symbol}_write_timeout");
    let no_pending_send = format!("{symbol}_no_pending_send");
    // bug-483: the two write-time terminal conditions route here to ask what
    // killed the session before naming the error.
    let write_classify = format!("{symbol}_write_classify");
    let peer_closed = format!("{symbol}_peer_closed");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([abi::load_u64(&v9, abi::return_register(), REC_CLOSED)]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CONN),
        abi::store_u64(&v9, abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX),
        // bug-564 (owner decision 2026-09-25): a `tls::read` on this socket has
        // already reported the peer's close. Raise before anything is created
        // or sent, empty payload included. Network.framework would take the
        // bytes of a half-closed peer, and the other backends refuse them too.
        abi::add_immediate(&v10, &v9, CTX_RXEOF),
        abi::load_acquire_u64(&v10, &v10),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&peer_closed),
    ]);
    // bug-497 / bug-508: one payload view for every backend — the text form
    // as before, the byte form after a header check (`push_write_payload_view`).
    let bad_payload = format!("{symbol}_bad_payload");
    push_write_payload_view(
        &mut ins,
        text,
        abi::c_arg(1),
        &v10,
        &v11,
        &v14,
        &v12,
        &v13,
        DLEN,
        DATA,
        &bad_payload,
    );
    // Empty payload: nothing to send.
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), DLEN),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&empty),
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
    // plan-110-D: a send whose deadline elapsed is still outstanding — its
    // completion block holds the CTX_SEM that was current when it was posted, and
    // `nw_connection_send` has no cancel. Drain it before `emit_fresh_sem`
    // replaces that semaphore, or the completion would signal an object nothing
    // waits on and this send would consume a stale count (the bug-52/55 hazard
    // the fresh-sem invariant exists to prevent). Bounded by the same deadline,
    // so a peer that never drains cannot wedge the next write either.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_WARMED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&no_pending_send),
    ]);
    emit_wait_bounded(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        DEADLINE,
        CTX_SEM,
        CTX_WTO,
        "wrdrain",
        &write_timeout,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_WARMED),
        abi::label(&no_pending_send),
    ]);
    emit_fresh_sem(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // bug-386: skip the send + FOREVER wait if the connection is already in a
    // terminal state (failed=4 / cancelled=5). The send completion would be
    // dropped and, with no further state transition to fire the state-changed
    // handler, the wait would hang. A write to a dead connection is an error,
    // not success. See the read path for the full rationale (the state handler
    // still covers a mid-wait transition).
    //
    // bug-483: which error it is depends on what killed the session, so this
    // goes through `write_classify` rather than straight to `ErrTlsFailed`. It
    // has to: measured against a departed peer, only the FIRST failing write
    // takes the CTX_ERROR path below — every write after it lands here, because
    // Network.framework has moved the connection to `failed` (4; never 5) by
    // then. Reporting `ErrConnectionClosed` once and `ErrTlsFailed` forever
    // after, for one disconnect, is what this bug is about.
    //
    // bug-564: this is a GATE read and `write_classify` reads the payload
    // (`CTX_EDOM`) after it, both unlocked against the handler thread. The
    // handler publishes the payload first and the gate by store-release. The
    // gate is loaded by load-acquire, which runs before every later load, so the
    // `CTX_EDOM` load cannot run early against memory older than the gate.
    //
    // bug-564: this guard runs BEFORE `dispatch_data_create`. It used to run
    // after it, and `write_classify`'s exits do not release the content, so
    // every write to a dead connection leaked the copy of its payload that
    // `dispatch_data_create` had just made.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::add_immediate(&v10, &v9, CTX_STATE),
        abi::load_acquire_u32(&v10, &v10),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&write_classify),
    ]);
    // content = dispatch_data_create(data, len, NULL, NULL)  (NULL = copy)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_data_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DATA),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DLEN),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONTENT),
    ]);
    // ctxdef = *_nw_content_context_default_message
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "_nw_content_context_default_message",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, &v9, 0),
        abi::store_u64(&v9, abi::stack_pointer(), CTXDEF),
    ]);
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        SEND_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // nw_connection_send(conn, content, context, is_complete=true, &block)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_send",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CONTENT),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), CTXDEF),
        abi::move_immediate(abi::c_arg(3), "Integer", "1"),
        abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Outstanding until its completion signals; see the drain above.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, CTX_WARMED),
    ]);
    emit_wait_bounded(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        DEADLINE,
        CTX_SEM,
        CTX_WTO,
        "wr",
        &write_timeout,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_WARMED),
    ]);
    // Release the content we created.
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
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONTENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // A non-null error means the send failed. bug-564: a gate, so
        // load-acquire, for the reason given at the terminal-state guard.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::add_immediate(&v10, &v9, CTX_ERROR),
        abi::load_acquire_u64(&v10, &v10),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&write_classify),
        abi::label(&empty),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // plan-110-D: the send deadline elapsed. CTX_WARMED stays 1 and CTX_SEM is
    // left alone — the send is still in flight and the next write drains it.
    // The content is deliberately NOT released here: `nw_connection_send` still
    // owns a reference until its completion runs.
    ins.push(abi::label(&write_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    // bug-483: name the failure from the domain the send completion recorded
    // (CTX_EDOM, written by SEND_INVOKE while the `nw_error` was still alive).
    // `nw_error_domain_posix` = 1 is the transport underneath the session going
    // away -- EPIPE/ECONNRESET -- which `tcp::write`, `tcp::read` and
    // `tls::read` all already call `ErrConnectionClosed`, and which
    // `mfb man tls write` promises. A `dns`(2)/`tls`(3) domain, or no error
    // classified at all (0), keeps the `ErrTlsFailed` this always reported: the
    // guard is NOT reclassified wholesale, only when a POSIX-domain send error
    // was actually observed on this connection.
    ins.push(abi::label(&write_classify));
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_EDOM),
        abi::compare_immediate(&v10, "1"), // nw_error_domain_posix
        abi::branch_eq(&peer_closed),
    ]);
    ins.push(abi::label(&write_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&peer_closed));
    emit_fail(symbol, "ErrConnectionClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed_moved));
    emit_fail(symbol, "ErrResourceMoved", &mut ins, &mut rel, &done);
    if !text {
        // bug-497: the byte form was handed a block whose header is not a
        // `List OF Byte`'s — refuse rather than read a length out of its bytes.
        ins.push(abi::label(&bad_payload));
        emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    }
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
