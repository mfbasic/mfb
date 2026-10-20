//! plan-176-T: the Network.framework `tls::read` lowering (`lower_tls_read_macos`). The shared
//! imports live in `builtins/tls/gen_macos/client/mod.rs`.

use super::*;

pub(crate) fn lower_tls_read_macos(
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
    let v15 = vregs.next();
    const FRAME_SIZE: usize = 192;
    const REC: usize = 8;
    const CONN: usize = 16;
    const CTX: usize = 24;
    const MAX: usize = 32;
    const HANDLE: usize = 40;
    const FNPTR: usize = 48;
    const MAPPED: usize = 64;
    const MPTR: usize = 72;
    const MSIZE: usize = 80;
    const N: usize = 88;
    const STR: usize = 96;
    const BLOCK: usize = 104; // 104..144
    const PBUF: usize = 144; // plan-76-B: scratch for the drain-armed arena copy
    const DEADLINE: usize = 152; // plan-110-D: computed dispatch_time for the read wait

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");
    // plan-76-B Phase 4: drain a poll-armed receive into CTX_PEND, then serve any
    // buffered plaintext before falling back to a fresh receive.
    let check_pend = format!("{symbol}_check_pend");
    let drain_map = format!("{symbol}_drain_map");
    let drain_copy_loop = format!("{symbol}_drain_copy_loop");
    let drain_copy_done = format!("{symbol}_drain_copy_done");
    let drain_publish = format!("{symbol}_drain_publish");
    let serve_pending = format!("{symbol}_serve_pending");
    let build_pending_n = format!("{symbol}_build_pending_n");
    let build_start = format!("{symbol}_build_start");
    let recv_path = format!("{symbol}_recv_path");
    let served_pending = format!("{symbol}_served_pending");
    let result_ready = format!("{symbol}_result_ready");
    // plan-110-D: the read deadline installed by `tls::setReadTimeout`.
    let read_timeout = format!("{symbol}_read_timeout");
    let drain_wait = format!("{symbol}_drain_wait");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
    ]);
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
        abi::load_u64(&v10, abi::stack_pointer(), MAX),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&invalid),
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
    // plan-76-B Phase 4: if a poll receive is outstanding (armed), consume it into
    // CTX_PEND first; then serve any buffered plaintext before posting a fresh
    // receive. When nothing is armed and CTX_PEND is empty (the no-poll case), this
    // falls straight through to `recv_path` — the existing behaviour, byte-for-byte.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_ARMED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&check_pend),
        abi::label(&drain_wait),
    ]);
    // plan-110-D: bounded by the socket's read deadline (CTX_RTO, unbounded
    // sentinel until `tls::setReadTimeout` installs one — so an unconfigured
    // socket still waits forever). On expiry the receive stays ARMED and its
    // completion block keeps its claim on CTX_PSEM/CTX_PCONTENT, so the NEXT
    // read consumes that same receive rather than posting a second one. That is
    // what makes a timed-out read resumable instead of leaking an outstanding
    // receive.
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
        CTX_PSEM,
        CTX_RTO,
        "rd",
        &read_timeout,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_ARMED),
        abi::load_u64(&v10, &v9, CTX_PCONTENT),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&peer_closed), // armed receive returned EOF
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
        "dispatch_data_create_map",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::label(&drain_map),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), MPTR),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), MSIZE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), MSIZE),
        abi::store_u64(&v9, abi::stack_pointer(), N),
        abi::move_register(abi::return_register(), &v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PBUF),
        abi::load_u64(&v9, abi::stack_pointer(), MPTR),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::load_u64(&v11, abi::stack_pointer(), N),
        abi::move_immediate(&v12, "Integer", "0"),
        abi::label(&drain_copy_loop),
        abi::compare_registers(&v12, &v11),
        abi::branch_eq(&drain_copy_done),
        abi::load_u8(&v13, &v9, 0),
        abi::store_u8(&v13, &v10, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v10, &v10, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&drain_copy_loop),
        abi::label(&drain_copy_done),
        abi::label(&drain_publish),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::store_u64(&v10, &v9, CTX_PEND_BUF),
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::store_u64(&v10, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
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
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        // Serve the buffered plaintext (or fall to the normal receive path).
        abi::label(&check_pend),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::compare_registers(&v10, &v11),
        abi::branch_ge(&recv_path),
        abi::label(&serve_pending),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::subtract_registers(&v11, &v11, &v10), // avail = len - off
        abi::load_u64(&v12, abi::stack_pointer(), MAX),
        abi::compare_registers(&v11, &v12),
        abi::branch_le(&build_pending_n),
        abi::move_register(&v11, &v12), // n = min(avail, maxBytes)
        abi::label(&build_pending_n),
        abi::store_u64(&v11, abi::stack_pointer(), N),
        abi::load_u64(&v12, &v9, CTX_PEND_BUF),
        abi::add_registers(&v12, &v12, &v10),
        abi::store_u64(&v12, abi::stack_pointer(), MPTR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MAPPED),
        abi::branch(&build_start),
        abi::label(&recv_path),
    ]);
    // No `emit_fresh_sem` here any more. Read waits on CTX_PSEM now, so
    // recycling CTX_SEM would do nothing for it — and would be actively wrong:
    // `tls::write` may have a send outstanding against the current CTX_SEM
    // (CTX_WARMED), and replacing it underneath that send is the exact
    // stale-semaphore hazard the fresh-sem invariant exists to prevent.
    // ctx->retain = &dispatch_retain (used inside the receive block).
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
        // plan-110-D: the fresh receive now uses the POLL block, not RECV_INVOKE.
        // Both blocks post the same `nw_connection_receive`; the difference is
        // where the completion lands — RECV_POLL_INVOKE stashes into
        // CTX_PCONTENT and signals CTX_PSEM, which is the pair that survives
        // across calls. That matters once a read can time out: the posted
        // receive cannot be cancelled, so its completion must have somewhere to
        // land that the NEXT read will look at. Using one outstanding-receive
        // model for both read and poll also collapses two drains into one.
        RECV_POLL_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // bug-386: if the connection has already transitioned to a terminal state
    // (failed=4 / cancelled=5), the async receive we are about to post will have
    // its completion block dropped by Network.framework, and — since no further
    // state transition will fire the state-changed handler — nothing would ever
    // signal the semaphore, hanging the wait below. Route to peer-closed (EOF),
    // matching how this path already surfaces a receive error. A transition that
    // happens DURING the wait still fires the state handler, so the pre-check
    // plus the handler close the hang on both sides of the race. CTX_STATE is
    // reset to 0 (invalid, < 4) at ctx setup and only the state handler raises
    // it, so a live/ready connection is never short-circuited.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&peer_closed),
    ]);
    // nw_connection_receive(conn, min=1, max=maxBytes, &block)
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
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MAX),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Mark it outstanding and join the single bounded drain, which owns the
        // wait, the deadline, and the CTX_PCONTENT handling for both the
        // just-posted receive and one a previous `tls::poll` armed.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, CTX_ARMED),
        abi::branch(&drain_wait),
    ]);
    // plan-110-D: every read now reaches here from the serve-from-CTX_PEND path
    // with MPTR/N set and MAPPED == 0. The fresh-receive path used to arrive here
    // with a live `dispatch_data` map instead; it now arms the receive and joins
    // the drain, which does its own mapping and releases before publishing into
    // CTX_PEND. One shape reaches the build, so there is one release policy.
    ins.push(abi::label(&build_start));
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::move_immediate(&v11, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v12, &v10, &v11),
        abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &v12, &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), STR),
        abi::move_immediate(&v9, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KIND),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&v9, "Byte", "1"),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&v11, abi::mfb_return(1), COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v12, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::load_u64(&v15, abi::stack_pointer(), MPTR),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&entry_done),
        // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
        // zero stride would rewrite one entry over the data region `count`
        // times and run past the block, so it is skipped outright.
    ]);
    if byte_list_entry_stride() != 0 {
        ins.extend([
            abi::move_immediate(&v12, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&v12, &v11, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(&v9, &v11, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&v12, "Integer", "1"),
            abi::store_u64(&v12, &v11, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    // The payload copy runs for BOTH representations.
    ins.extend([
        abi::add_registers(&v12, &v14, &v9),
        abi::load_u8(&v13, &v15, 0),
        abi::store_u8(&v13, &v12, 0),
        abi::add_immediate(&v15, &v15, 1),
        abi::add_immediate(&v11, &v11, byte_list_entry_stride()),
        abi::add_immediate(&v9, &v9, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
    ]);

    // Served from CTX_PEND: advance the consume cursor; free + clear the buffer
    // when fully drained so a later poll never overwrites (and leaks) it. There
    // is no NW object to release here — the drain already released the map and
    // the retained content before copying into CTX_PEND.
    ins.extend([
        abi::label(&served_pending),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), N),
        abi::add_registers(&v10, &v10, &v11),
        abi::store_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::compare_registers(&v10, &v11),
        abi::branch_lt(&result_ready),
        abi::load_u64(abi::return_register(), &v9, CTX_PEND_BUF),
        abi::load_u64(abi::c_arg(1), &v9, CTX_PEND_LEN),
    ]);
    emit_arena_free(symbol, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_BUF),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
        abi::label(&result_ready),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // bug-564: this read has reported the peer's close, so every later
    // `tls::write` must raise (see `CTX_RXEOF`). Both routes here (a final empty
    // receive, a terminal state) load CTX into the frame first.
    ins.extend([
        abi::label(&peer_closed),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::add_immediate(&v9, &v9, CTX_RXEOF),
        abi::store_release_u64(&v10, &v9),
    ]);
    emit_fail(symbol, "ErrConnectionClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    // plan-110-D: the read deadline elapsed. CTX_ARMED is deliberately still 1
    // and CTX_PCONTENT untouched -- the receive is still outstanding and the
    // next read picks it up. Nothing is released here: releasing the content or
    // clearing ARMED would strand the completion block.
    ins.push(abi::label(&read_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed_moved));
    emit_fail(symbol, "ErrResourceMoved", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
