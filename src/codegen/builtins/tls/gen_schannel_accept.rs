// Included into gen_schannel.rs, after `gen_schannel_server.rs` (plan-176-T). The server-path
// accept helpers: accept (accept + the AcceptSecurityContext handshake) and closeListener
// (FreeCredentialsHandle + closesocket); listen and its credential helpers are in
// `gen_schannel_server.rs`.

// ASC_REQ_REPLAY_DETECT|SEQUENCE_DETECT|CONFIDENTIALITY|ALLOCATE_MEMORY|
// EXTENDED_ERROR|STREAM = 4+8+0x10+0x100+0x8000+0x10000 = 98588. The STREAM bit is
// 0x10000 for ASC (0x8000 for ISC) — the flag values differ between the two calls.
const ASC_REQ_FLAGS: &str = "98588";
// Wait for an inbound connection: WSAPoll(POLLRDNORM).
const POLLRDNORM: &str = "256";

/// bug-739: re-arm the handshake's socket timeouts to the time left before the
/// deadline at `sp + deadline_off`, or branch to `expired` when none is left.
/// A no-op when `timeoutMs` (at `sp + timeout_off`) is the unbounded sentinel.
/// `tag` keeps the call sites' labels apart.
#[allow(clippy::too_many_arguments)]
fn emit_hs_refresh(symbol: &str, tag: &str, timeout_off: usize, deadline_off: usize, fd_off: usize, tv_off: usize, expired: &str, imports: &HashMap<String, String>, platform: &dyn CodegenPlatform, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>, vregs: &mut Vregs) -> Result<(), String> {
    let t = vregs.next();
    let s = vregs.next();
    let skip = format!("{symbol}_hs_refresh_{tag}_skip");
    ins.extend([
        abi::load_u64(&t, abi::stack_pointer(), timeout_off),
        abi::move_immediate(&s, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&t, &s),
        abi::branch_eq(&skip),
    ]);
    crate::codegen::os::ffi::emit_external_int_call(platform, "GetTickCount64", symbol, 0, imports, ins, rel)?;
    ins.extend([
        abi::load_u64(&t, abi::stack_pointer(), deadline_off),
        abi::subtract_registers(&t, &t, abi::return_register()),
        abi::compare_immediate(&t, "0"),
        abi::branch_le(expired),
        abi::store_u64(&t, abi::stack_pointer(), tv_off),
    ]);
    super::gen_shared::emit_set_sock_timeouts(&mut EmitCtx { symbol, platform_imports: imports, platform, instructions: ins, relocations: rel }, fd_off, tv_off)?;
    ins.push(abi::label(&skip));
    Ok(())
}

// ---------------------------------------------------------------------------
// tls.accept(listener, timeoutMs) -> Socket
// ---------------------------------------------------------------------------
pub(crate) fn lower_tls_accept(
    symbol: &str,
    imports: &HashMap<String, String>,
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
    let v6 = vregs.next();
    let v18 = vregs.next();
    const LCRED: usize = 8; // &listener.WORK.CRED (arena absolute)
    const TIMEOUT: usize = 16;
    const LISTENFD: usize = 24;
    const CONNFD: usize = 32;
    const STATE: usize = 40; // per-connection STATE ptr (st::*)
    const FIRSTF: usize = 48; // 0 until the first ASC has created the context
    const POLLFD: usize = 56; // WSAPOLLFD { SOCKET fd; SHORT events; SHORT revents }
    const HSTV: usize = 64; // plan-73-D: handshake SO_*TIMEO DWORD-ms scratch
    const HSTOF: usize = 72; // plan-73-D: 1 if the handshake recv timed out (WSAETIMEDOUT)
    const DEADLINE: usize = 80; // bug-739: GetTickCount64 ms by which the handshake must finish
    const FRAME_SIZE: usize = 0x100;

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let no_timeout = format!("{symbol}_no_timeout");
    let accept_invalid = format!("{symbol}_accept_invalid");
    let accept_ts_store = format!("{symbol}_accept_ts_clamped");
    let accept_fail = format!("{symbol}_accept_fail");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let tls_fail = format!("{symbol}_tls_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let hs_read = format!("{symbol}_hs_read");
    let hs_asc = format!("{symbol}_hs_asc");
    let hs_done = format!("{symbol}_hs_done");
    let hs_finish = format!("{symbol}_hs_finish");
    let have_ctx = format!("{symbol}_have_ctx");
    let arg1_done = format!("{symbol}_arg1_done");
    let no_send = format!("{symbol}_no_send");
    let resetrecv = format!("{symbol}_resetrecv");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    // return_register = listener record { fd@0, closed@8, WORK@16 }; ARG[1] = timeoutMs.
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOF),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
    ]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    ins.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), LISTENFD),
        abi::load_u64(&v9, abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK), // WORK ptr
        abi::add_immediate(&v9, &v9, stl::CRED),
        abi::store_u64(&v9, abi::stack_pointer(), LCRED),
        // plan-73-D: the unbounded sentinel => a blocking accept (omit = block); `0`
        // => WSAPoll(0), one immediate attempt (`ErrTimeout` if none pending); `> 0`
        // => WSAPoll(timeoutMs); a negative (non-sentinel) => ErrInvalidArgument.
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&no_timeout),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&accept_invalid),
        // Clamp `> 0` to INT_MAX and store it back — WSAPoll takes a C `int`, so a
        // value with bit 31 set would be read as a block-forever timeout; net clamps
        // identically. WSAPoll and the handshake SO_*TIMEO both reload TIMEOUT.
        abi::move_immediate(&v10, "Integer", "2147483647"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&accept_ts_store),
        abi::move_register(&v9, &v10),
        abi::label(&accept_ts_store),
        abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT),
        abi::load_u64(&v9, abi::stack_pointer(), LISTENFD),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD),
        abi::move_immediate(&v10, "Integer", POLLRDNORM),
        abi::store_u16(&v10, abi::stack_pointer(), POLLFD + 8),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), POLLFD + 10),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT),
    ]);
    platform.emit_external_call("WSAPoll", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::branch_eq(&accept_timeout),
        abi::label(&no_timeout),
        // accept(fd, NULL, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENFD),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_external_call("accept", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONNFD),
    ]);
    // plan-73-D / bug-739: bound the WHOLE handshake by one deadline, taken now
    // that the connection is accepted: DEADLINE = GetTickCount64() + timeoutMs.
    // Before every handshake recv and send, `emit_hs_refresh` re-installs
    // SO_RCVTIMEO/SO_SNDTIMEO as the time LEFT, so a peer that trickles bytes
    // cannot reset the bound with each one. Sentinel => unbounded (omit); `0` =>
    // a 1 ms deadline (Winsock reads a 0 DWORD as infinite), one near-immediate
    // attempt. Cleared after the handshake (hs_done) so the returned socket's
    // read/write are unbounded.
    {
        let hs_ts_ok = format!("{symbol}_hs_ts_ok");
        let hs_ts_skip = format!("{symbol}_hs_ts_skip");
        ins.extend([
            abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT),
            abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v14, &v15),
            abi::branch_eq(&hs_ts_skip),
        ]);
        crate::codegen::os::ffi::emit_external_int_call(platform, "GetTickCount64", symbol, 0, imports, &mut ins, &mut rel)?;
        ins.extend([
            abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT),
            abi::compare_immediate(&v14, "0"),
            abi::branch_ne(&hs_ts_ok),
            abi::move_immediate(&v14, "Integer", "1"),
            abi::label(&hs_ts_ok),
            abi::add_registers(&v14, &v14, abi::return_register()),
            abi::store_u64(&v14, abi::stack_pointer(), DEADLINE),
            abi::label(&hs_ts_skip),
        ]);
    }

    // Allocate the per-connection STATE block (zeroed header) + mark it server-side.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &st::SIZE.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), STATE));
    ins.push(abi::move_register(&v10, abi::mfb_return(1)));
    for o in (0..st::RECV).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, &v10, o));
    }
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FIRSTF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u32(&v9, &v10, st::SERVER),
        // recv_len = 0
        abi::store_u64(abi::ZERO, &v10, st::RECV_LEN),
    ]);

    // Handshake: recv (client sends ClientHello first), AcceptSecurityContext, send.
    let hs_expired = format!("{symbol}_hs_expired");
    ins.push(abi::label(&hs_read));
    emit_hs_refresh(symbol, "r", TIMEOUT, DEADLINE, CONNFD, HSTV, &hs_expired, imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD),
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u64(&v11, &v10, st::RECV_LEN),
        abi::add_immediate(abi::c_arg(1), &v10, st::RECV),
        abi::add_registers(abi::c_arg(1), abi::c_arg(1), &v11),
        abi::move_immediate(abi::c_arg(2), "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::c_arg(2), abi::c_arg(2), &v11),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_external_call("recv", symbol, imports, &mut ins, &mut rel)?;
    let hs_got = format!("{symbol}_hs_got");
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&hs_got), // got handshake bytes
    ]);
    // plan-73-D: recv <= 0 — an SO_RCVTIMEO expiry is WSAETIMEDOUT (10060), a
    // handshake TIMEOUT → ErrTimeout (via the flag); a peer close or other error
    // stays ErrTlsFailed.
    platform.emit_errno(symbol, (&v9).into(), imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(&v9, "10060"), // WSAETIMEDOUT
        abi::branch_ne(&tls_fail),
        // bug-739: no time left before a recv or send is the same timeout.
        abi::label(&hs_expired),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), HSTOF),
        abi::branch(&tls_fail),
        abi::label(&hs_got),
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u64(&v11, &v10, st::RECV_LEN),
        abi::add_registers(&v11, &v11, abi::return_register()),
        abi::store_u64(&v11, &v10, st::RECV_LEN),
        abi::label(&hs_asc),
    ]);
    // bug-739: this pass may send a token; give the send only the time left.
    emit_hs_refresh(symbol, "s", TIMEOUT, DEADLINE, CONNFD, HSTV, &hs_expired, imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.extend([
        // in SecBuffer[0] = {recv_len, TOKEN, &RECV}; [1] = {0, EMPTY, NULL}
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u64(&v11, &v10, st::RECV_LEN),
        abi::store_u32(&v11, &v10, st::INBUF),
        abi::move_immediate(&v9, "Integer", SECBUFFER_TOKEN),
        abi::store_u32(&v9, &v10, st::INBUF + 4),
        abi::add_immediate(&v9, &v10, st::RECV),
        abi::store_u64(&v9, &v10, st::INBUF + 8),
    ]);
    set_secbuffer(&v10, st::INBUF + 16, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins, &mut vregs);
    set_secbuffer_desc(&v10, st::INDESC, "2", st::INBUF, &mut ins, &mut vregs);
    set_secbuffer(&v10, st::OUTBUF, "0", SECBUFFER_TOKEN, abi::ZERO, &mut ins, &mut vregs);
    set_secbuffer_desc(&v10, st::OUTDESC, "1", st::OUTBUF, &mut ins, &mut vregs);
    // phContext = FIRST ? NULL : &ctxt
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LCRED), // 0: phCredential
        abi::load_u64(&v9, abi::stack_pointer(), FIRSTF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&have_ctx),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::branch(&arg1_done),
        abi::label(&have_ctx),
        abi::load_u64(&v9, abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(1), &v9, st::CTXT),
        abi::label(&arg1_done),
        // arg2 = &indesc; arg3 = ASC flags
        abi::load_u64(&v9, abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(2), &v9, st::INDESC),
        abi::move_immediate(abi::c_arg(3), "Integer", ASC_REQ_FLAGS),
    ]);
    // ASC(&cred, phContext, &indesc, flags, 0, &ctxt, &outdesc, &attrs, &expiry)
    sspi_call_ext(
        symbol,
        "AcceptSecurityContext",
        STATE,
        &[None, Some(st::CTXT), Some(st::OUTDESC), Some(st::ATTRS), Some(st::EXPIRY)],
        imports,
        platform,
        &mut ins,
        &mut rel,
     &mut vregs)?;
    ins.push(abi::move_register(&v15, abi::return_register()));
    // SEC_E_INCOMPLETE_MESSAGE: read more and call again. This must come BEFORE
    // FIRSTF is set: an incomplete first call creates no context, so the retry
    // must pass NULL again. Setting FIRSTF first handed AcceptSecurityContext a
    // handle it never filled in, and a ClientHello that arrived in more than one
    // piece failed with ErrTlsFailed (bug-748).
    branch_if_incomplete(&v15, &hs_read, &mut ins, &mut vregs);
    ins.extend([
        // context now exists → subsequent calls pass &ctxt
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), FIRSTF),
        abi::compare_immediate(&v15, "0"),
        abi::branch_lt(&tls_fail),
    ]);
    emit_send_token(symbol, CONNFD, STATE, st::OUTBUF, &no_send, "atok", &tls_fail, imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.push(abi::label(&no_send));
    ins.extend([
        abi::compare_immediate(&v15, SEC_E_OK),
        abi::branch_eq(&hs_finish),
        // SEC_I_CONTINUE_NEEDED: handle SECBUFFER_EXTRA in INBUF[1] or recv anew.
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u32(&v9, &v10, st::INBUF + 16 + 4),
        abi::compare_immediate(&v9, SECBUFFER_EXTRA),
        abi::branch_ne(&resetrecv),
        abi::load_u32(&v11, &v10, st::INBUF + 16),
        abi::load_u64(&v12, &v10, st::RECV_LEN),
        abi::subtract_registers(&v13, &v12, &v11),
        abi::add_immediate(&v14, &v10, st::RECV),
        abi::add_registers(&v14, &v14, &v13),
        abi::add_immediate(&v6, &v10, st::RECV),
    ]);
    move_bytes(&v14, &v6, &v11, &format!("{symbol}_aextra"), &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u32(&v11, &v10, st::INBUF + 16),
        abi::store_u64(&v11, &v10, st::RECV_LEN),
        abi::branch(&hs_asc),
        abi::label(&resetrecv),
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, &v10, st::RECV_LEN),
        abi::branch(&hs_read),
        // Handshake complete: the final ASC consumed the client's last flight from
        // RECV. If application data arrived coalesced (INBUF[1] EXTRA), keep it at
        // the front of RECV for the first read; otherwise reset RECV_LEN to 0 so
        // read does not re-decrypt consumed handshake bytes.
        abi::label(&hs_finish),
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u32(&v9, &v10, st::INBUF + 16 + 4),
        abi::compare_immediate(&v9, SECBUFFER_EXTRA),
        abi::branch_ne(&format!("{symbol}_fin_noextra")),
        abi::load_u32(&v11, &v10, st::INBUF + 16),
        abi::load_u64(&v12, &v10, st::RECV_LEN),
        abi::subtract_registers(&v13, &v12, &v11),
        abi::add_immediate(&v14, &v10, st::RECV),
        abi::add_registers(&v14, &v14, &v13),
        abi::add_immediate(&v6, &v10, st::RECV),
    ]);
    move_bytes(&v14, &v6, &v11, &format!("{symbol}_finextra"), &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u32(&v11, &v10, st::INBUF + 16),
        abi::store_u64(&v11, &v10, st::RECV_LEN),
        abi::branch(&hs_done),
        abi::label(&format!("{symbol}_fin_noextra")),
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, &v10, st::RECV_LEN),
        abi::label(&hs_done),
    ]);

    // plan-73-D: handshake done — clear SO_*TIMEO so the returned socket's read/write
    // stay unbounded. Only `0`/`> 0` installed a timeout; the sentinel left it unset.
    {
        let hs_clr_skip = format!("{symbol}_hs_clr_skip");
        ins.extend([
            abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT),
            abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v14, &v15),
            abi::branch_eq(&hs_clr_skip),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTV),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTV + 8),
        ]);
        super::gen_shared::emit_set_sock_timeouts(
            &mut EmitCtx {
                symbol,
                platform_imports: imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            CONNFD,
            HSTV,
        )?;
        ins.push(abi::label(&hs_clr_skip));
    }

    // QueryContextAttributes(&ctxt, STREAM_SIZES, &sizes) → header/trailer/max.
    // &sizes reuses the per-connection SC_CRED scratch (unused server-side).
    ins.extend([
        abi::load_u64(&v18, abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), &v18, st::CTXT),
        abi::move_immediate(abi::c_arg(1), "Integer", SECPKG_ATTR_STREAM_SIZES),
        abi::add_immediate(abi::c_arg(2), &v18, st::SC_CRED),
    ]);
    sspi_call(symbol, "QueryContextAttributesW", SECUR32, 3, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&tls_fail));
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u32(&v9, &v10, st::SC_CRED),
        abi::store_u32(&v9, &v10, st::HEADER),
        abi::load_u32(&v9, &v10, st::SC_CRED + 4),
        abi::store_u32(&v9, &v10, st::TRAILER),
        abi::load_u32(&v9, &v10, st::SC_CRED + 8),
        abi::store_u32(&v9, &v10, st::MAXMSG),
    ]);

    // Build the Socket record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the tail { SSPI block ptr @TLS_SCHANNEL_OFFSET_BLOCK } (plan-80).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_SCHANNEL),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), CONNFD),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), STATE),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    ins.push(abi::label(&tls_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD));
    platform.emit_external_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // plan-73-D: a handshake recv that hit the SO_RCVTIMEO (WSAETIMEDOUT) is a
    // timeout → ErrTimeout; every other tls_fail is a TLS failure.
    let tls_fail_timeout = format!("{symbol}_tls_fail_timeout");
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HSTOF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&tls_fail_timeout),
    ]);
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&tls_fail_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument (rejected
    // up front, before any accept/alloc).
    ins.push(abi::label(&accept_invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed_moved));
    emit_fail(symbol, "ErrResourceMoved", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);

    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME_SIZE))
}

// ---------------------------------------------------------------------------
// tls.closeListener(listener)
// ---------------------------------------------------------------------------
pub(crate) fn lower_tls_close_listener(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const REC: usize = 8;
    const FD: usize = 16;
    const WORK: usize = 24;
    const FRAME_SIZE: usize = 0x40;

    let already = format!("{symbol}_already");
    let already_moved = format!("{symbol}_already_moved");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        // Idempotent: a closed handle returns OK.
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
    ]);
    ins.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &already,
        &already_moved,
    ));
    ins.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD),
        abi::load_u64(&v9, abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::store_u64(&v9, abi::stack_pointer(), WORK),
        // FreeCredentialsHandle(&WORK.CRED)
        abi::load_u64(&v9, abi::stack_pointer(), WORK),
        abi::add_immediate(abi::return_register(), &v9, stl::CRED),
    ]);
    sspi_call(symbol, "FreeCredentialsHandle", SECUR32, 1, imports, platform, &mut ins, &mut rel)?;
    // CertFreeCertificateContext(WORK.CERTPTR)
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), &v9, stl::CERTPTR),
    ]);
    win_call(symbol, "CertFreeCertificateContext", 1, false, imports, platform, &mut ins, &mut rel)?;
    // CryptDestroyKey(WORK.HKEY); CryptReleaseContext(WORK.HPROV, 0)
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), &v9, stl::HKEY),
    ]);
    win_call(symbol, "CryptDestroyKey", 1, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), &v9, stl::HPROV),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    win_call(symbol, "CryptReleaseContext", 2, false, imports, platform, &mut ins, &mut rel)?;
    // Delete the persisted keyset container (best effort):
    // CryptAcquireContextW(&HPROV, CONTNAME, NULL, PROV_RSA_AES, CRYPT_DELETEKEYSET).
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), WORK),
        abi::add_immediate(abi::return_register(), &v9, stl::HPROV),
        abi::add_immediate(abi::c_arg(1), &v9, stl::CONTNAME),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "24"),
        abi::move_immediate(abi::c_arg(4), "Integer", "16"), // CRYPT_DELETEKEYSET
    ]);
    win_call(symbol, "CryptAcquireContextW", 5, false, imports, platform, &mut ins, &mut rel)?;
    // closesocket(fd)
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_external_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // bug-623: return the WORK block (credential, certificate, key container) to
    // the arena; the releases above were its last use, `tls::accept` checks the
    // closed flag before reading it, and an accepted socket keeps no pointer into
    // it. A transferred listener's WORK is the receiver's own arena copy
    // (`SlotTransfer::ArenaBlock`). The slot is zeroed after the free.
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), WORK),
        abi::move_immediate(abi::c_arg(1), "Integer", &LISTENER_BLOCK_SIZE.to_string()),
    ]);
    crate::codegen::engine::builder::emit_arena_free(symbol, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::store_u64(abi::ZERO, &v9, TLS_SCHANNEL_OFFSET_BLOCK),
    ]);
    // Mark the record closed.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, TLS_LISTENER_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&already),
    ]);
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
    Ok((ins, rel, FRAME_SIZE))
}
