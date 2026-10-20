//! plan-176-T: the Network.framework `tls::poll` lowering (`lower_tls_poll_macos`). The shared
//! imports live in `builtins/tls/gen_macos/client/mod.rs`.

use super::*;

/// plan-76-B Phase 4: `tls::poll(sock[, timeoutMs]) AS Boolean` on macOS
/// Network.framework, via the outstanding-receive model (Corrections
/// B-macos-blocker / B-macos-blocker-2). NW has no non-blocking data-readiness
/// query and a posted receive cannot be cancelled, so readiness is driven by an
/// ISOLATED poll receive: it posts one `nw_connection_receive` whose completion
/// (RECV_POLL_INVOKE) stashes into `CTX_PCONTENT` and signals the dedicated
/// `CTX_PSEM` — never the read/write `CTX_SEM` — and on completion this helper
/// copies the mapped bytes into a persistent arena buffer (`CTX_PEND_*`) that
/// `tls::read` drains first. A bounded/zero-timeout poll that expires leaves the
/// receive outstanding (`CTX_ARMED`) so its bytes are stashed by the next
/// poll/read rather than lost. Readable = buffered plaintext present, OR the
/// outstanding receive has completed with bytes, OR the connection is terminal
/// (EOF/error). `x0` = sock record, `x1` = timeoutMs.
pub(crate) fn lower_tls_poll_macos(
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
    const FRAME_SIZE: usize = 160;
    const REC: usize = 8;
    const TIMEOUT: usize = 16;
    const CONN: usize = 24;
    const CTX: usize = 32;
    const HANDLE: usize = 40;
    const FNPTR: usize = 48;
    const DEADLINE: usize = 56;
    const MPTR: usize = 64;
    const MSIZE: usize = 72;
    const MAPPED: usize = 80;
    const N: usize = 88;
    const PBUF: usize = 96;
    const BLOCK: usize = 104; // 104..144

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let invalid = format!("{symbol}_invalid");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let skip_neg = format!("{symbol}_skip_neg");
    let do_wait = format!("{symbol}_do_wait");
    let wait_forever = format!("{symbol}_wait_forever");
    let wait_now = format!("{symbol}_wait_now");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let stash = format!("{symbol}_stash");
    let stash_release = format!("{symbol}_stash_release");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
    ]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    ins.extend([
        abi::load_u64(&v9, abi::return_register(), REC_CONN),
        abi::store_u64(&v9, abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX),
        // Reject a genuine negative timeout (the unbounded sentinel is allowed).
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&skip_neg),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::label(&skip_neg),
        // Fast path: undelivered buffered plaintext already present → readable now.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::compare_registers(&v10, &v11),
        abi::branch_lt(&ready),
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
    ins.extend([
        // A receive already outstanding (a prior bounded poll timed out) → just wait
        // on it; do not post another.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_ARMED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&do_wait),
        // Terminal connection (failed=4 / cancelled=5) → readable (read returns EOF).
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&ready),
    ]);
    // ctx->retain = &dispatch_retain (the poll block retains its content).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_retain",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v10, &v9, CTX_RETAIN),
        // Reset the poll receive's output slots before posting.
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_PERROR),
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
        RECV_POLL_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        // bug-386 style pre-check: a terminal connection would drop the receive's
        // completion, so route to readable (EOF) rather than arm a receive that
        // never signals.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&ready),
    ]);
    // nw_connection_receive(conn, min=1, max=65536, &block); mark armed.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_receive",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_immediate(abi::c_arg(2), "Integer", "65536"),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, CTX_ARMED),
        // Compute the wait deadline (connect-path policy): sentinel→FOREVER, 0→NOW,
        // >0→dispatch_time(NOW, ms*1e6).
        abi::label(&do_wait),
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
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
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_time",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(&v10, "Integer", "1000000"),
        abi::multiply_registers(abi::c_arg(1), abi::c_arg(1), &v10),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_now),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9), // DISPATCH_TIME_FOREVER
        abi::store_u64(&v9, abi::stack_pointer(), DEADLINE),
        abi::label(&deadline_ready),
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
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PSEM),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DEADLINE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Non-zero => the deadline elapsed; the receive stays outstanding (armed)
        // for the next poll/read to consume. Not ready.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&not_ready),
        // Signaled: the poll receive completed. Clear armed, stash its content.
        abi::label(&stash),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_ARMED),
        abi::load_u64(&v10, &v9, CTX_PCONTENT),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&ready), // null content = EOF/terminal → readable
    ]);
    // dispatch_data_create_map(content, &ptr, &size) -> mapped
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_data_create_map",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), MPTR),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), MSIZE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), MSIZE),
        abi::store_u64(&v9, abi::stack_pointer(), N),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&stash_release), // 0-byte map: nothing to buffer
        // Copy the mapped bytes into a persistent arena buffer so no NW object is
        // held across the poll→read boundary.
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), N),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PBUF),
        // memcpy(PBUF, MPTR, N)
        abi::load_u64(&v9, abi::stack_pointer(), MPTR),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::load_u64(&v11, abi::stack_pointer(), N),
        abi::move_immediate(&v12, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v12, &v11),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v13, &v9, 0),
        abi::store_u8(&v13, &v10, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v10, &v10, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // Publish the pending buffer.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::store_u64(&v10, &v9, CTX_PEND_BUF),
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::store_u64(&v10, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
        abi::label(&stash_release),
    ]);
    // Release the mapped data and the retained content.
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
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        abi::branch(&ready),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed_moved));
    emit_fail(symbol, "ErrResourceMoved", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
