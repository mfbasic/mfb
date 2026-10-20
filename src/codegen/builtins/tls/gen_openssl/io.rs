//! plan-176-T: the OpenSSL `tls::read`, `tls::poll` and `tls::write` lowerings. The shared imports live in
//! `builtins/tls/gen_openssl/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// tls.read (bytes only; plan-110-D removed tls.readText)
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_read_openssl(
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
    const FRAME_SIZE: usize = 96;
    const SSL_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;
    // bug-623: the finished List OF Byte, spilled across the read-buffer free.
    const RESULT_OFFSET: usize = 56;
    // bug-564: the record, so the peer_closed exit can set its flag.
    const REC_OFFSET: usize = 64;

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let read_fail = format!("{symbol}_read_fail");
    let read_ok = format!("{symbol}_read_ok");
    let read_timeout = format!("{symbol}_read_timeout");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    // bug-623: an allocation failure once the read buffer exists (the result
    // list) releases the buffer first; `alloc_fail` is the buffer's own.
    let result_alloc_fail = format!("{symbol}_result_alloc_fail");
    // bug-623: release the maxBytes read buffer. Every exit after its allocation
    // runs this exactly once -- the success path after the copy, each failure
    // label before raising. Pointer and size reload from the frame: the
    // dlsym/SSL_read/alloc calls clobber every caller-saved register.
    let emit_free_read_buffer = |instructions: &mut Vec<CodeInstruction>| {
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), BUF_OFFSET),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), MAX_OFFSET),
            abi::branch_link(ARENA_FREE_SYMBOL),
        ]);
    };
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
    ]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&invalid),
        // Allocate a maxBytes read buffer.
        abi::move_register(abi::return_register(), &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        BUF_OFFSET,
    ));
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_read",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    // n = SSL_read(ssl, buf, maxBytes)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_read returns a C int; sign-extend before the signed 0/<0 tests so a
        // -1 error isn't read as a large positive byte count (bug-102).
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        // Spill `n` BEFORE the branches. Both successors need it -- the success
        // path as the byte count, the failure path as `SSL_get_error`'s second
        // argument -- and spilling once here means nothing downstream reads the
        // aligned bank across an intervening external call. The `read_ok` label
        // is emitted after the classification block below, so a linear scan from
        // that block's `blr` would otherwise reach a `str_u64 rdi` that is only
        // ever reached by the `b.gt` above it. The scan is what
        // `assert_no_aligned_bank_result_reads` does (bug-452), and rather than
        // teach it control flow the value is simply not left live in `rdi`.
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_gt(&read_ok),
    ]);
    // plan-110-D: distinguish the socket's read deadline from a transport or
    // protocol failure. `tls::setReadTimeout` installs SO_RCVTIMEO; when it
    // expires the underlying `read(2)` returns EAGAIN/EWOULDBLOCK, which
    // OpenSSL surfaces as SSL_ERROR_WANT_READ (2) or SSL_ERROR_WANT_WRITE (3)
    // — the session is intact and the call may simply be retried. The contract
    // says a deadline raises ErrTimeout on every platform; without this the
    // Linux backend reported ErrTlsFailed while macOS reported ErrTimeout for
    // the identical event, and a caller could not tell a slow peer from a
    // broken session.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_get_error",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), N_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::sign_extend_word(&v10, abi::c_return(0)),
        abi::compare_immediate(&v10, "2"), // SSL_ERROR_WANT_READ
        abi::branch_eq(&read_timeout),
        abi::compare_immediate(&v10, "3"), // SSL_ERROR_WANT_WRITE
        abi::branch_eq(&read_timeout),
        // bug-564: a peer that went away without close_notify (reset, or the
        // process died) is the peer's close, exactly as on macOS and Windows and
        // as `tls::write` already classifies it here (bug-467). OpenSSL says so
        // three ways: SSL_ERROR_SYSCALL (5, ECONNRESET), SSL_ERROR_ZERO_RETURN
        // (6), and on 3.x SSL_ERROR_SSL (1) with reason
        // SSL_R_UNEXPECTED_EOF_WHILE_READING (294). Every other SSL_ERROR_SSL is
        // a protocol failure and stays ErrTlsFailed.
        abi::compare_immediate(&v10, "5"), // SSL_ERROR_SYSCALL
        abi::branch_eq(&peer_closed),
        abi::compare_immediate(&v10, "6"), // SSL_ERROR_ZERO_RETURN
        abi::branch_eq(&peer_closed),
        abi::compare_immediate(&v10, "1"), // SSL_ERROR_SSL
        abi::branch_ne(&read_fail),
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
        "ERR_peek_last_error",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::move_register(&v10, abi::c_return(0)),
        abi::move_immediate(&v11, "Integer", "8388607"), // ERR_REASON_MASK (0x7FFFFF)
        abi::and_registers(&v10, &v10, &v11),
        abi::compare_immediate(&v10, "294"), // SSL_R_UNEXPECTED_EOF_WHILE_READING
        abi::branch_eq(&peer_closed),
        abi::branch(&read_fail),
        abi::label(&read_ok),
    ]);
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
        abi::move_immediate(&v11, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v12, &v10, &v11),
        abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &v12, &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        symbol,
        &mut instructions,
        &mut relocations,
        &result_alloc_fail,
    );
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(&v9, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KIND),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&v9, "Byte", "1"),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&v11, abi::mfb_return(1), COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v12, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::load_u64(&v15, abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&entry_done),
        // A packed fixed-width byte list (kind 2, plan-57-D) has NO entry
        // array: its stride is 0 and the data region starts right after the
        // header. Writing 40-byte entry descriptors here put an entry-flags
        // byte (COLLECTION_ENTRY_FLAG_USED == 1) where element 0's payload
        // belongs, so `tls::read` handed back a list whose first byte was
        // always 1. Mirrors the guard in `gen_macos/client.rs`.
    ]);
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
    // The payload copy runs for BOTH representations.
    instructions.extend([
        abi::add_registers(&v12, &v14, &v9),
        abi::load_u8(&v13, &v15, 0),
        abi::store_u8(&v13, &v12, 0),
        abi::add_immediate(&v15, &v15, 1),
        abi::add_immediate(&v11, &v11, byte_list_entry_stride()),
        abi::add_immediate(&v9, &v9, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
    ]);
    emit_free_read_buffer(&mut instructions);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // bug-564 (owner decision 2026-09-25): this read reported the peer's close,
    // so every later `tls::write` on the socket raises (`TLS_OFFSET_PEER_CLOSED`).
    instructions.extend([
        abi::label(&peer_closed),
        abi::load_u64(&v9, abi::stack_pointer(), REC_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, TLS_OFFSET_PEER_CLOSED),
    ]);
    emit_free_read_buffer(&mut instructions);
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&read_timeout));
    emit_free_read_buffer(&mut instructions);
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&read_fail));
    emit_free_read_buffer(&mut instructions);
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // Every `load_fail` branch is taken after the buffer allocation (libssl is
    // resolved once the buffer exists), so it releases the buffer too.
    instructions.push(abi::label(&load_fail));
    emit_free_read_buffer(&mut instructions);
    emit_fail(
        symbol,
        "ErrTlsFailed",
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
    instructions.push(abi::label(&closed_moved));
    emit_fail(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&result_alloc_fail));
    emit_free_read_buffer(&mut instructions);
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // One declaration covers every `arena_free` site above (per (from, to) pair).
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.poll  (plan-76-B: tls::poll(sock[, timeoutMs]) AS Boolean)
// ---------------------------------------------------------------------------

/// TLS readiness on OpenSSL. `readable = SSL_pending(ssl) > 0 OR poll(fd, POLLIN)`:
/// the `SSL_pending` fast-path catches decrypted app bytes already buffered in the
/// TLS layer with the fd idle (which an fd-only poll would miss), and the `poll(2)`
/// fallback carries the plan-73 timeout (sentinel→block, `<0`→invalid, `>0`→clamp
/// `INT_MAX`, EINTR-retry — the `net::poll` policy). `x0` = sock record, `x1` =
/// timeoutMs. Returns `Boolean`.
pub(crate) fn lower_tls_poll_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 64;
    const TIMEOUT_OFFSET: usize = 8;
    const SSL_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;
    const POLLFD_OFFSET: usize = 48; // pollfd { fd; events; revents }

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let invalid = format!("{symbol}_invalid");
    let load_fail = format!("{symbol}_load_fail");
    let poll_infinite = format!("{symbol}_poll_infinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let poll_retry = format!("{symbol}_poll_retry");
    let poll_fail = format!("{symbol}_poll_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
    ]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        // Save the fd now — the record pointer in x0 is clobbered by the SSL_pending
        // call below, so the fd for the poll fallback must come from a stack slot.
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        // Normalize the timeout (net::poll policy): sentinel→-1 (block), <0→invalid,
        // >0→clamp INT_MAX. Store the effective value back.
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&poll_infinite),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v10, "Integer", "2147483647"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&timeout_ok),
        abi::move_register(&v9, &v10),
        abi::branch(&timeout_ok),
        abi::label(&poll_infinite),
        abi::bitwise_not(&v9, abi::ZERO),
        abi::label(&timeout_ok),
        abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    // SSL_pending fast-path: buffered decrypted bytes => readable now (skip poll).
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_pending",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_pending returns a C int count; sign-extend before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&ready),
        // No buffered bytes: poll the raw fd (reloaded from its stack slot, since the
        // record pointer in x0 was clobbered by the SSL_pending call). Build
        // pollfd{ fd, POLLIN }.
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"), // POLLIN
        abi::store_u8(&v10, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::label(&poll_retry),
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
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_fail),
        abi::branch_eq(&not_ready),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_fail),
    ]);
    // EINTR → re-issue the poll; any other errno → resource-closed (the readiness
    // check failed, matching net::poll's hard-error class).
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, "4"), // EINTR
        abi::branch_eq(&poll_retry),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
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
    instructions.push(abi::label(&closed_moved));
    emit_fail(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, FRAME_SIZE))
}

// ---------------------------------------------------------------------------
// tls.write / tls.writeText
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_write_openssl(
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
    const FRAME_SIZE: usize = 80;
    const SSL_OFFSET: usize = 8;
    const SRC_OFFSET: usize = 16;
    const REMAINING_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;
    // bug-467: `SSL_write`'s return, spilled before the failure branch so the
    // classification below can hand it to `SSL_get_error` without reading the
    // aligned result bank across an external call (bug-452, and the same reason
    // `lower_tls_read_openssl` spills its `n`).
    const N_OFFSET: usize = 48;

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let load_fail = format!("{symbol}_load_fail");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_fail = format!("{symbol}_write_fail");
    let write_classify = format!("{symbol}_write_classify");
    let write_timeout = format!("{symbol}_write_timeout");
    let peer_closed = format!("{symbol}_peer_closed");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([abi::load_u64(
        &v9,
        abi::return_register(),
        TLS_OFFSET_CLOSED,
    )]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        // bug-564 (owner decision 2026-09-25): a `tls::read` on this socket has
        // already reported the peer's close. Raise before touching the session,
        // empty payload included; OpenSSL would otherwise hand the bytes to a
        // half-closed peer.
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_PEER_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&peer_closed),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
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
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_write",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::label(&write_loop),
        abi::load_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&write_done),
        // n = SSL_write(ssl, src, remaining)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SRC_OFFSET),
        abi::move_register(abi::c_arg(2), &v10),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_write returns a C int; sign-extend before the signed <=0 test (bug-102).
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_classify),
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
    ]);
    // bug-467: classify the failure instead of collapsing every one of them into
    // `ErrTlsFailed`. Until this bug, a write to a peer that had gone away never
    // got here at all -- libssl's internal `write(2)` delivered SIGPIPE and the
    // process died -- so the only reachable failures were load/protocol ones and
    // one blanket code was enough. With SIGPIPE ignored the peer's disappearance
    // now arrives here as a return value, and `tcp` and `tls` are documented
    // drop-in mirrors: `tcp::write` raises `ErrConnectionClosed` for it, and
    // `tls::read`/`tcp::read` already agree on that code at end of stream
    // (bug-465, rt-behavior/{tcp,tls}/*-read-eof-raises-rt). `SSL_get_error` is
    // the only way to tell the three cases apart:
    //
    //   2/3  WANT_READ / WANT_WRITE -- the SO_SNDTIMEO deadline `tls::setWriteTimeout`
    //        installs expired. The session is intact; the convention is ErrTimeout
    //        (plan-110-D). Letting this fall into the closed-connection arm would
    //        report a slow peer as a broken one -- a new bug, not an old one.
    //   5/6  SYSCALL / ZERO_RETURN -- the transport is gone (EPIPE, ECONNRESET) or
    //        the peer sent close_notify. Mirrors `tcp::write`, which maps every
    //        errno that is not EAGAIN/EINTR to ErrConnectionClosed.
    //   else the protocol failure `ErrTlsFailed` has always meant.
    //
    // Reusing FNPTR_OFFSET for `SSL_get_error` is safe because every arm below is
    // terminal: nothing re-enters `write_loop`, which is the only reader of the
    // `SSL_write` pointer. `lower_tls_read_openssl` does exactly the same.
    instructions.push(abi::label(&write_classify));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_get_error",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), N_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::sign_extend_word(&v10, abi::c_return(0)),
        abi::compare_immediate(&v10, "2"), // SSL_ERROR_WANT_READ
        abi::branch_eq(&write_timeout),
        abi::compare_immediate(&v10, "3"), // SSL_ERROR_WANT_WRITE
        abi::branch_eq(&write_timeout),
        abi::compare_immediate(&v10, "5"), // SSL_ERROR_SYSCALL
        abi::branch_eq(&peer_closed),
        abi::compare_immediate(&v10, "6"), // SSL_ERROR_ZERO_RETURN
        abi::branch_eq(&peer_closed),
    ]);
    instructions.push(abi::label(&write_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&write_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
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
