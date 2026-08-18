// Included into schannel.rs. The client-path helpers: connect (handshake),
// read (DecryptMessage), write (EncryptMessage), close (shutdown). listen/accept
// (server) are a separate future surface and stay unadvertised on Windows.

/// Blocking socket connect to host:port. Leaves the connected fd in `fd_off`;
/// branches to `fail` on any failure. Uses the portable Winsock getaddrinfo/
/// socket/connect (imported via ws2_32). Scratch frame slots hints/res/hostcstr
/// are caller-provided.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn socket_connect(
    symbol: &str,
    host_off: usize,
    port_off: usize,
    hints_off: usize,
    res_off: usize,
    hostcstr_off: usize,
    fd_off: usize,
    // plan-73-D: timeout support. `timeout_off` holds the timeoutMs; the unbounded
    // sentinel => a WSAPoll timeout of -1 (INFINITE, omit=block), `0` => 0 (one
    // immediate attempt), `> 0` => that many ms. `connect_timeout` is taken when the
    // WSAPoll deadline elapses before the socket becomes writable.
    timeout_off: usize,
    connect_timeout: &str,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Scratch in the caller's connect frame (0x100), above the last used slot (REC=128).
    const CFLAGS: usize = 200; // saved socket flags (unused on Winsock ioctlsocket)
    const CPOLLFD: usize = 208; // WSAPOLLFD { SOCKET fd@0; SHORT events@8; SHORT revents@10 }
    const CSOERR: usize = 224; // getsockopt SO_ERROR out
    const CSOLEN: usize = 232; // getsockopt optlen
    let nb_connected = format!("{symbol}_sc_connected");
    let have_to = format!("{symbol}_sc_have_to");
    let addr_off = platform.addrinfo_addr_offset();
    // hints: zero 48 bytes, ai_family=AF_INET, ai_socktype=SOCK_STREAM.
    for o in (0..48).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), hints_off + o));
    }
    ins.extend([
        abi::move_immediate("%v9", "Integer", super::HINTS_FAMILY_WORD),
        abi::store_u64("%v9", abi::stack_pointer(), hints_off),
        abi::move_immediate("%v9", "Integer", super::SOCK_STREAM),
        abi::store_u64("%v9", abi::stack_pointer(), hints_off + 8),
    ]);
    super::emit_cstring(symbol, "h", host_off, hostcstr_off, fail, ins, rel);
    // getaddrinfo(host, NULL, &hints, &res)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hostcstr_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), hints_off),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), res_off),
    ]);
    platform.emit_libc_call("getaddrinfo", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        // socket(res->ai_family, ai_socktype, ai_protocol)
        abi::load_u64("%v9", abi::stack_pointer(), res_off),
        abi::load_u32(abi::return_register(), "%v9", 4),
        abi::load_u32(abi::c_arg(1), "%v9", 8),
        abi::load_u32(abi::c_arg(2), "%v9", 12),
    ]);
    platform.emit_libc_call("socket", symbol, imports, ins, rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        // Overwrite sin_port at ai_addr+2/3 with the requested port (network order).
        abi::load_u64("%v9", abi::stack_pointer(), res_off),
        abi::load_u64("%v9", "%v9", addr_off),
        abi::load_u64("%v10", abi::stack_pointer(), port_off),
        abi::shift_right_immediate("%v11", "%v10", 8),
        abi::store_u8("%v11", "%v9", 2),
        abi::store_u8("%v10", "%v9", 3),
    ]);
    // plan-73-D: non-blocking connect + WSAPoll so tls::connect honors timeoutMs.
    // ioctlsocket(fd, FIONBIO, &1).
    platform.emit_set_nonblocking(fd_off, CFLAGS, symbol, imports, ins, rel)?;
    // connect(fd, ai_addr, ai_addrlen) — returns SOCKET_ERROR/WSAEWOULDBLOCK when in
    // progress, or 0 if it completed at once (localhost).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::load_u64("%v9", abi::stack_pointer(), res_off),
        abi::load_u64(abi::c_arg(1), "%v9", addr_off),
        abi::load_u32(abi::c_arg(2), "%v9", 16),
    ]);
    platform.emit_libc_call("connect", symbol, imports, ins, rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&nb_connected),
    ]);
    // Anything other than "in progress" (WSAEWOULDBLOCK) is a hard failure.
    platform.emit_errno(symbol, ("%v9").into(), imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate("%v9", platform.socket_in_progress_code()),
        abi::branch_ne(fail),
        // effectiveTimeout = sentinel ? -1 (INFINITE) : timeoutMs (0 = immediate).
        abi::load_u64("%v9", abi::stack_pointer(), timeout_off),
        abi::move_immediate("%v10", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ne(&have_to),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::bitwise_not("%v9", "%v9"), // -1 = INFINITE
        abi::label(&have_to),
        abi::store_u64("%v9", abi::stack_pointer(), CSOLEN), // stash effectiveTimeout
        // WSAPoll(&WSAPOLLFD { fd; events = POLLWRNORM; revents }, 1, effectiveTimeout)
        abi::load_u64("%v9", abi::stack_pointer(), fd_off),
        abi::store_u64("%v9", abi::stack_pointer(), CPOLLFD),
        abi::move_immediate("%v10", "Integer", "16"), // POLLWRNORM (0x0010)
        abi::store_u16("%v10", abi::stack_pointer(), CPOLLFD + 8),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), CPOLLFD + 10),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), CPOLLFD),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), CSOLEN),
    ]);
    platform.emit_libc_call("WSAPoll", symbol, imports, ins, rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(fail),
        abi::branch_eq(connect_timeout),
        // getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len) — 0 => connected.
        abi::move_immediate("%v9", "Integer", "4"),
        abi::store_u64("%v9", abi::stack_pointer(), CSOLEN),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CSOERR),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
        abi::move_immediate(abi::c_arg(2), "Integer", platform.so_error()),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), CSOERR),
        abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), CSOLEN),
    ]);
    platform.emit_libc_call("getsockopt", symbol, imports, ins, rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(fail),
        abi::load_u32("%v9", abi::stack_pointer(), CSOERR),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(fail),
        abi::label(&nb_connected),
    ]);
    // Restore blocking mode: ioctlsocket(fd, FIONBIO, &0).
    platform.emit_restore_blocking(fd_off, CFLAGS, symbol, imports, ins, rel)?;
    // freeaddrinfo(res)
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), res_off));
    platform.emit_libc_call("freeaddrinfo", symbol, imports, ins, rel)?;
    Ok(())
}

/// send(fd, buf, len, 0) the whole buffer (loop until len bytes sent). Branches
/// to `fail` on a send error. buf/len are register operands (consumed).
fn send_all(
    symbol: &str,
    fd_off: usize,
    buf: &str,
    len: &str,
    tag: &str,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let loop_l = format!("{symbol}_{tag}_send");
    let done_l = format!("{symbol}_{tag}_sent");
    // %v6 = remaining, %v7 = cursor
    ins.extend([
        abi::move_register("%v6", len),
        abi::move_register("%v7", buf),
        abi::label(&loop_l),
        abi::compare_immediate("%v6", "0"),
        abi::branch_le(&done_l),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::move_register(abi::c_arg(1), "%v7"),
        abi::move_register(abi::c_arg(2), "%v6"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_libc_call("send", symbol, imports, ins, rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(fail),
        abi::add_registers("%v7", "%v7", abi::return_register()),
        abi::subtract_registers("%v6", "%v6", abi::return_register()),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
    Ok(())
}

// A SecBuffer is { u32 cbBuffer; u32 BufferType; u64 pvBuffer } = 16 bytes.
// A SecBufferDesc is { u32 ulVersion; u32 cBuffers; u64 pBuffers } = 16 bytes.
// `base` is the register holding the block base (the stack pointer for the read
// path, the arena STATE pointer for connect/close — see `st::` scratch).
fn set_secbuffer(base: &str, off: usize, cb: &str, ty: &str, ptr_reg: &str, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::move_immediate("%v9", "Integer", cb),
        abi::store_u32("%v9", base, off),
        abi::move_immediate("%v9", "Integer", ty),
        abi::store_u32("%v9", base, off + 4),
        abi::store_u64(ptr_reg, base, off + 8),
    ]);
}

fn set_secbuffer_desc(base: &str, off: usize, count: &str, buffers_off: usize, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::move_immediate("%v9", "Integer", SECBUFFER_VERSION),
        abi::store_u32("%v9", base, off),
        abi::move_immediate("%v9", "Integer", count),
        abi::store_u32("%v9", base, off + 4),
        abi::add_immediate("%v9", base, buffers_off),
        abi::store_u64("%v9", base, off + 8),
    ]);
}

pub(crate) fn lower_tls_connect(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Connect frame locals. The SCHANNEL_CRED (0x60), SecBuffers, and the two
    // SecBufferDescs live on the stack; the per-connection STATE is in the arena.
    const HOST: usize = 8;
    const PORT: usize = 16;
    const TIMEOUT: usize = 24;
    const SNAME: usize = 32;
    const SNAMEW: usize = 40; // wide serverName cstr ptr
    const HINTS: usize = 48; // 48..96
    const RES: usize = 96;
    const HOSTCSTR: usize = 104;
    const FD: usize = 112;
    const STATE: usize = 120; // arena state ptr
    const REC: usize = 128; // resource record ptr
    const HSTV: usize = 240; // plan-73-D: handshake SO_*TIMEO DWORD-ms scratch
    const HSTOF: usize = 248; // plan-73-D: 1 if the handshake recv timed out (WSAETIMEDOUT)
    // The SCHANNEL_CRED, SecBuffers, SecBufferDescs, attrs and expiry all live in
    // the arena STATE block (st::SC_CRED/OUTBUF/OUTDESC/INBUF/INDESC/ATTRS/EXPIRY),
    // so their pointers are absolute and survive sspi_call_ext's sub_sp (see there).
    const FRAME_SIZE: usize = 0x100;

    let fail = format!("{symbol}_fail");
    // Socket-level (TCP connect / WSAPoll / getsockopt) failures are network
    // failures; the TLS handshake/verify failures at `fail` are ErrTlsFailed. This
    // split matches the OpenSSL connect backend and this backend's own accept path
    // (both distinguish ErrNetworkFailed from ErrTlsFailed), where connect formerly
    // reported every failure as ErrNetworkFailed.
    let net_fail = format!("{symbol}_net_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connect_invalid = format!("{symbol}_connect_invalid");
    let done = format!("{symbol}_done");
    let hs_loop = format!("{symbol}_hs_loop");
    let hs_read = format!("{symbol}_hs_read");
    let hs_done = format!("{symbol}_hs_done");
    let hs_finish = format!("{symbol}_hs_finish");
    let no_token = format!("{symbol}_no_token");
    let no_extra = format!("{symbol}_no_extra");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), SNAME),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOF),
    ]);
    {
        // plan-73-D: reject a negative (non-sentinel) `timeoutMs` up front — before
        // getaddrinfo/socket. The omitted overload pads the unbounded sentinel
        // (allowed → an INFINITE WSAPoll + unbounded handshake); `0`/`> 0` pass on.
        let ts_ok = format!("{symbol}_ts_ok");
        let ts_store = format!("{symbol}_ts_clamped");
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
            abi::move_immediate("%v10", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_eq(&ts_ok),
            abi::compare_immediate("%v9", "0"),
            abi::branch_lt(&connect_invalid),
            // Clamp `> 0` to INT_MAX and store it back: WSAPoll takes a C `int`, so a
            // value with bit 31 set would be read as a block-forever (-1) timeout;
            // net clamps identically. Both socket_connect (WSAPoll) and the handshake
            // SO_*TIMEO reload TIMEOUT, so both see the clamped value. Sentinel skips.
            abi::move_immediate("%v10", "Integer", "2147483647"),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_le(&ts_store),
            abi::move_register("%v9", "%v10"),
            abi::label(&ts_store),
            abi::store_u64("%v9", abi::stack_pointer(), TIMEOUT),
            abi::label(&ts_ok),
        ]);
    }

    socket_connect(symbol, HOST, PORT, HINTS, RES, HOSTCSTR, FD, TIMEOUT, &connect_timeout, &net_fail, imports, platform, &mut ins, &mut rel)?;

    // plan-73-D: bound the TLS handshake recv by SO_RCVTIMEO/SO_SNDTIMEO. The
    // unbounded sentinel => leave it unbounded (omit = block); `0` => the smallest
    // nonzero wait (tv_usec 1µs, near-immediate); `> 0` => the timeval. Cleared after
    // the handshake so the returned socket's read/write stay unbounded.
    {
        let hs_ts_ok = format!("{symbol}_hs_ts_ok");
        let hs_ts_skip = format!("{symbol}_hs_ts_skip");
        ins.extend([
            abi::load_u64("%v14", abi::stack_pointer(), TIMEOUT),
            abi::move_immediate("%v15", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers("%v14", "%v15"),
            abi::branch_eq(&hs_ts_skip),
            // Winsock SO_*TIMEO is a DWORD of milliseconds (not a timeval); a value of
            // 0 means infinite, so the convention's `0` (non-blocking) uses 1 ms.
            abi::compare_immediate("%v14", "0"),
            abi::branch_ne(&hs_ts_ok),
            abi::move_immediate("%v14", "Integer", "1"),
            abi::label(&hs_ts_ok),
            abi::store_u64("%v14", abi::stack_pointer(), HSTV),
        ]);
        super::emit_set_sock_timeouts(
            &mut EmitCtx {
                symbol,
                platform_imports: imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            FD,
            HSTV,
        )?;
        ins.push(abi::label(&hs_ts_skip));
    }

    // Allocate the arena STATE block (zeroed) + the 32-byte resource record.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &st::SIZE.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), STATE));
    // zero the state header (through RECV start) so leftover/recv_len start clean.
    ins.push(abi::move_register("%v10", abi::mfb_return(1)));
    for o in (0..st::RECV).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, "%v10", o));
    }
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), REC),
        // Canonical plan-80 header: tag@0, fd@8, closed@16, STATE@24 (the SSPI
        // credential/context block ptr lives in the tail at TLS_SCHANNEL_OFFSET_BLOCK).
        abi::move_immediate("%v9", "Integer", RESOURCE_TAG_TLS_SCHANNEL),
        abi::store_u64("%v9", abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64("%v9", abi::stack_pointer(), FD),
        abi::store_u64("%v9", abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
    ]);

    // Build SCHANNEL_CRED at STATE+SC_CRED { dwVersion=4; dwFlags=AUTO|STRONG }.
    // The whole STATE header (0..RECV) was already zeroed above, so only the two
    // non-zero fields are written. dwFlags is at offset 72 (0x48) in the x64
    // SCHANNEL_CRED (after grbitEnabledProtocols/cipher strengths/session lifespan);
    // grbitEnabledProtocols stays 0 (system default).
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), STATE),
        abi::move_immediate("%v9", "Integer", SCHANNEL_CRED_VERSION),
        abi::store_u32("%v9", "%v18", st::SC_CRED),
        abi::move_immediate("%v9", "Integer", SCH_CRED_FLAGS),
        abi::store_u32("%v9", "%v18", st::SC_CRED + 72),
    ]);
    // Marshal serverName -> wide cstr (SNAMEW) for pszTargetName.
    emit_wide_cstring(symbol, SNAME, SNAMEW, &alloc_fail, imports, platform, &mut ins, &mut rel)?;

    // AcquireCredentialsHandleW(NULL, "Microsoft Unified Security Protocol
    //   Provider", SECPKG_CRED_OUTBOUND, NULL, &cred, NULL, NULL, &state.cred, &expiry)
    // Register args 0..3 set directly; stack args 4..8 are arena offsets.
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "0")); // 0: pszPrincipal=NULL
    wide_addr(symbol, abi::c_arg(1), USP_NAME, &mut ins, &mut rel); // 1: pszPackage
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", SECPKG_CRED_OUTBOUND), // 2
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),                  // 3: pvLogonID=NULL
    ]);
    // stack args 4..8: &SCHANNEL_CRED, NULL, NULL, &state.cred, &expiry (arena)
    sspi_call_ext(
        symbol,
        "AcquireCredentialsHandleW",
        STATE,
        &[Some(st::SC_CRED), None, None, Some(st::CRED), Some(st::EXPIRY)],
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::branch_lt(&fail));

    // --- Handshake: first ISC with no input token ---
    // out SecBuffer[0] = {0, TOKEN, NULL}; ALLOCATE_MEMORY makes Schannel fill it.
    ins.push(abi::load_u64("%v18", abi::stack_pointer(), STATE));
    set_secbuffer("%v18", st::OUTBUF, "0", SECBUFFER_TOKEN, &abi_zero(), &mut ins);
    set_secbuffer_desc("%v18", st::OUTDESC, "1", st::OUTBUF, &mut ins);
    // ISC(&cred, NULL, sname, flags, 0, 0, NULL, 0, &ctxt, &outdesc, &attrs, &expiry)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CRED), // 0: phCredential
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),         // 1: phContext=NULL
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), SNAMEW), // 2: pszTargetName
        abi::move_immediate(abi::c_arg(3), "Integer", ISC_REQ_FLAGS), // 3: fContextReq
    ]);
    // stack args 4..11: 0,0,NULL,0, &ctxt, &outdesc, &attrs, &expiry (arena)
    sspi_call_ext(
        symbol,
        "InitializeSecurityContextW",
        STATE,
        &[None, None, None, None, Some(st::CTXT), Some(st::OUTDESC), Some(st::ATTRS), Some(st::EXPIRY)],
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    // Expect SEC_I_CONTINUE_NEEDED (positive); any negative status is a failure.
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&fail),
    ]);
    // send the token.
    emit_send_token(symbol, FD, STATE, st::OUTBUF, &no_token, "tok0", &fail, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&no_token));

    // --- Handshake loop: recv, ISC with input token, send output, repeat ---
    ins.extend([
        // recv_len = 0
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
        abi::label(&hs_read),
        // recv(fd, state.RECV + recv_len, RECV_CAP - recv_len, 0)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::add_immediate(abi::c_arg(1), "%v10", st::RECV),
        abi::add_registers(abi::c_arg(1), abi::c_arg(1), "%v11"),
        abi::move_immediate(abi::c_arg(2), "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::c_arg(2), abi::c_arg(2), "%v11"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_libc_call("recv", symbol, imports, &mut ins, &mut rel)?;
    let hs_got = format!("{symbol}_hs_got");
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&hs_got),
    ]);
    // plan-73-D: recv <= 0. An SO_RCVTIMEO expiry on Winsock is WSAETIMEDOUT (10060,
    // not EWOULDBLOCK) — that is a handshake TIMEOUT → ErrTimeout (via the flag);
    // anything else stays ErrNetworkFailed.
    platform.emit_errno(symbol, ("%v9").into(), imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate("%v9", "10060"), // WSAETIMEDOUT
        abi::branch_ne(&fail),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), HSTOF),
        abi::branch(&fail),
        abi::label(&hs_got),
        // recv_len += n
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::add_registers("%v11", "%v11", abi::return_register()),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::label(&hs_loop),
        // in SecBuffer[0] = {recv_len, TOKEN, &RECV}; [1] = {0, EMPTY, NULL}
    ]);
    // in SecBuffer[0] = {recv_len, TOKEN, &RECV} at STATE+INBUF; [1] = {0,EMPTY,NULL}.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::store_u32("%v11", "%v10", st::INBUF),
        abi::move_immediate("%v9", "Integer", SECBUFFER_TOKEN),
        abi::store_u32("%v9", "%v10", st::INBUF + 4),
        abi::add_immediate("%v9", "%v10", st::RECV),
        abi::store_u64("%v9", "%v10", st::INBUF + 8),
    ]);
    set_secbuffer("%v10", st::INBUF + 16, "0", SECBUFFER_EMPTY, &abi_zero(), &mut ins);
    set_secbuffer_desc("%v10", st::INDESC, "2", st::INBUF, &mut ins);
    set_secbuffer("%v10", st::OUTBUF, "0", SECBUFFER_TOKEN, &abi_zero(), &mut ins);
    set_secbuffer_desc("%v10", st::OUTDESC, "1", st::OUTBUF, &mut ins);
    // ISC(&cred, &ctxt, sname, flags, 0, 0, &indesc, 0, &ctxt, &outdesc, &attrs, &expiry)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CRED), // 0: phCredential
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(1), abi::c_arg(1), st::CTXT), // 1: phContext=&ctxt
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), SNAMEW), // 2: pszTargetName
        abi::move_immediate(abi::c_arg(3), "Integer", ISC_REQ_FLAGS), // 3: fContextReq
    ]);
    // stack args 4..11: 0,0,&indesc,0, &ctxt, &outdesc, &attrs, &expiry (arena)
    sspi_call_ext(
        symbol,
        "InitializeSecurityContextW",
        STATE,
        &[None, None, Some(st::INDESC), None, Some(st::CTXT), Some(st::OUTDESC), Some(st::ATTRS), Some(st::EXPIRY)],
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // %v15 = status
    ins.push(abi::move_register("%v15", abi::return_register()));
    // SEC_E_INCOMPLETE_MESSAGE → recv more (keep buffer).
    branch_if_incomplete("%v15", &hs_read, &mut ins);
    // Any negative status other than INCOMPLETE → handshake/cert failure.
    ins.extend([
        abi::compare_immediate("%v15", "0"),
        abi::branch_lt(&fail),
    ]);
    // Send any output token produced.
    emit_send_token(symbol, FD, STATE, st::OUTBUF, &no_extra, "tok1", &fail, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&no_extra));
    // If SEC_E_OK → finish; else (SEC_I_CONTINUE_NEEDED) reset recv_len and loop.
    ins.extend([
        abi::compare_immediate("%v15", SEC_E_OK),
        abi::branch_eq(&hs_finish),
        // handle SECBUFFER_EXTRA in INBUF[1]: move leftover to front, else recv anew.
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::INBUF + 16 + 4), // type of buf[1]
        abi::compare_immediate("%v9", SECBUFFER_EXTRA),
        abi::branch_ne(&format!("{symbol}_resetrecv")),
        // move extra bytes (buf[1].cbBuffer) from end to front of RECV.
        abi::load_u32("%v11", "%v10", st::INBUF + 16), // extra len
        abi::load_u64("%v12", "%v10", st::RECV_LEN),
        abi::subtract_registers("%v13", "%v12", "%v11"), // src offset
        abi::add_immediate("%v14", "%v10", st::RECV),
        abi::add_registers("%v14", "%v14", "%v13"), // src ptr
        abi::add_immediate("%v6", "%v10", st::RECV), // dst ptr (front)
    ]);
    move_bytes("%v14", "%v6", "%v11", &format!("{symbol}_extra"), &mut ins);
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::branch(&hs_loop),
        abi::label(&format!("{symbol}_resetrecv")),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
        abi::branch(&hs_read),
        // Handshake complete: the final ISC consumed the server's last flight from
        // RECV. Any coalesced post-handshake data (a TLS 1.3 NewSessionTicket, or
        // application data) is INBUF[1] SECBUFFER_EXTRA — keep it at the front of
        // RECV for the first read; otherwise reset RECV_LEN to 0 so read does not
        // re-decrypt consumed handshake bytes (which stranded the first read of a
        // TLS 1.2 server that filled RECV with its final flight).
        abi::label(&hs_finish),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::INBUF + 16 + 4),
        abi::compare_immediate("%v9", SECBUFFER_EXTRA),
        abi::branch_ne(&format!("{symbol}_fin_noextra")),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::load_u64("%v12", "%v10", st::RECV_LEN),
        abi::subtract_registers("%v13", "%v12", "%v11"),
        abi::add_immediate("%v14", "%v10", st::RECV),
        abi::add_registers("%v14", "%v14", "%v13"),
        abi::add_immediate("%v6", "%v10", st::RECV),
    ]);
    move_bytes("%v14", "%v6", "%v11", &format!("{symbol}_finextra"), &mut ins);
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::branch(&hs_done),
        abi::label(&format!("{symbol}_fin_noextra")),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
        abi::label(&hs_done),
    ]);

    // plan-73-D: handshake done — clear SO_*TIMEO so the returned socket's read/write
    // stay unbounded. Only `0`/`> 0` installed a timeout (the sentinel left it
    // unbounded), so only they clear it.
    {
        let hs_clr_skip = format!("{symbol}_hs_clr_skip");
        ins.extend([
            abi::load_u64("%v14", abi::stack_pointer(), TIMEOUT),
            abi::move_immediate("%v15", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers("%v14", "%v15"),
            abi::branch_eq(&hs_clr_skip),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTV),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTV + 8),
        ]);
        super::emit_set_sock_timeouts(
            &mut EmitCtx {
                symbol,
                platform_imports: imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            FD,
            HSTV,
        )?;
        ins.push(abi::label(&hs_clr_skip));
    }

    // QueryContextAttributes(&ctxt, STREAM_SIZES, &sizes) → header/trailer/max.
    // SecPkgContext_StreamSizes { u32 cbHeader; cbTrailer; cbMaximumMessage;
    //   cBuffers; cbBlockSize } — write cbHeader/cbTrailer/cbMax into state.
    // &sizes reuses the arena SC_CRED scratch (SCHANNEL_CRED no longer needed).
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), "%v18", st::CTXT),
        abi::move_immediate(abi::c_arg(1), "Integer", SECPKG_ATTR_STREAM_SIZES),
        abi::add_immediate(abi::c_arg(2), "%v18", st::SC_CRED),
    ]);
    sspi_call(symbol, "QueryContextAttributesW", SECUR32, 3, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::SC_CRED),
        abi::store_u32("%v9", "%v10", st::HEADER),
        abi::load_u32("%v9", "%v10", st::SC_CRED + 4),
        abi::store_u32("%v9", "%v10", st::TRAILER),
        abi::load_u32("%v9", "%v10", st::SC_CRED + 8),
        abi::store_u32("%v9", "%v10", st::MAXMSG),
    ]);

    // Enforce the HOSTNAME against the negotiated chain (bug: easy to omit).
    emit_verify_hostname(symbol, STATE, SNAMEW, &fail, imports, platform, &mut ins, &mut rel)?;

    // Store state ptr in the resource, return the resource.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v10", abi::stack_pointer(), REC),
        abi::store_u64("%v9", "%v10", TLS_SCHANNEL_OFFSET_BLOCK),
        abi::move_register(RESULT_VALUE_REGISTER, "%v10"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // plan-73-D: the non-blocking connect's WSAPoll deadline elapsed before the
    // socket became writable — close the pending socket + release the resolver
    // results, then report a timeout.
    ins.push(abi::label(&connect_timeout));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RES));
    platform.emit_libc_call("freeaddrinfo", symbol, imports, &mut ins, &mut rel)?;
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    // A negative (non-sentinel) `timeoutMs` → ErrInvalidArgument (rejected up front,
    // before getaddrinfo/socket, so nothing to clean up).
    ins.push(abi::label(&connect_invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    // plan-73-D: a handshake recv that hit the SO_RCVTIMEO (WSAETIMEDOUT) is a
    // timeout → ErrTimeout; every other `fail` branch is a network failure.
    let fail_timeout = format!("{symbol}_fail_timeout");
    ins.push(abi::label(&fail));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), HSTOF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&fail_timeout),
    ]);
    // A handshake/verify failure (not a handshake-recv timeout) => ErrTlsFailed,
    // matching the OpenSSL connect backend and this backend's accept path.
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&fail_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    // A socket-level (TCP connect / WSAPoll / getsockopt) failure => ErrNetworkFailed.
    ins.push(abi::label(&net_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}

fn abi_zero() -> String {
    // A register holding 0 for a SecBuffer pvBuffer=NULL: store ZERO directly.
    abi::ZERO.to_string()
}

/// Move `count` bytes from `[src]` to `[dst]` front-to-back (safe when dst<src).
fn move_bytes(src: &str, dst: &str, count: &str, tag: &str, ins: &mut Vec<CodeInstruction>) {
    let l = format!("{tag}_mv");
    let d = format!("{tag}_mvd");
    ins.extend([
        abi::move_immediate("%v5", "Integer", "0"),
        abi::label(&l),
        abi::compare_registers("%v5", count),
        abi::branch_eq(&d),
        abi::load_u8("%v4", src, 0),
        abi::store_u8("%v4", dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::add_immediate("%v5", "%v5", 1),
        abi::branch(&l),
        abi::label(&d),
    ]);
}

include!("schannel_io.rs");
