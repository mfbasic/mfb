// Included into schannel.rs. The client-path helpers: connect (handshake),
// read (DecryptMessage), write (EncryptMessage), close (shutdown). listen/accept
// (server) are a separate future surface and stay unadvertised on Windows.

/// Blocking socket connect to host:port. Leaves the connected fd in `fd_off`;
/// branches to `fail` on any failure. Uses the portable Winsock getaddrinfo/
/// socket/connect (imported via ws2_32). Scratch frame slots hints/res/hostcstr
/// are caller-provided.
#[allow(clippy::too_many_arguments)]
fn socket_connect(
    symbol: &str,
    host_off: usize,
    port_off: usize,
    hints_off: usize,
    res_off: usize,
    hostcstr_off: usize,
    fd_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), hints_off),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), res_off),
    ]);
    platform.emit_libc_call("getaddrinfo", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        // socket(res->ai_family, ai_socktype, ai_protocol)
        abi::load_u64("%v9", abi::stack_pointer(), res_off),
        abi::load_u32(abi::return_register(), "%v9", 4),
        abi::load_u32(abi::ARG[1], "%v9", 8),
        abi::load_u32(abi::ARG[2], "%v9", 12),
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
        // connect(fd, ai_addr, ai_addrlen)  — blocking.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::load_u64("%v9", abi::stack_pointer(), res_off),
        abi::load_u64(abi::ARG[1], "%v9", addr_off),
        abi::load_u32(abi::ARG[2], "%v9", 16),
    ]);
    platform.emit_libc_call("connect", symbol, imports, ins, rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), res_off),
    ]);
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
        abi::move_register(abi::ARG[1], "%v7"),
        abi::move_register(abi::ARG[2], "%v6"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
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

pub(super) fn lower_tls_connect(
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
    // The SCHANNEL_CRED, SecBuffers, SecBufferDescs, attrs and expiry all live in
    // the arena STATE block (st::SC_CRED/OUTBUF/OUTDESC/INBUF/INDESC/ATTRS/EXPIRY),
    // so their pointers are absolute and survive sspi_call_ext's sub_sp (see there).
    const FRAME_SIZE: usize = 0x100;

    let fail = format!("{symbol}_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let hs_loop = format!("{symbol}_hs_loop");
    let hs_read = format!("{symbol}_hs_read");
    let hs_done = format!("{symbol}_hs_done");
    let no_token = format!("{symbol}_no_token");
    let no_extra = format!("{symbol}_no_extra");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), SNAME),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE),
    ]);
    let _ = TIMEOUT;

    socket_connect(symbol, HOST, PORT, HINTS, RES, HOSTCSTR, FD, &fail, imports, platform, &mut ins, &mut rel)?;

    // Allocate the arena STATE block (zeroed) + the 32-byte resource record.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &st::SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), STATE));
    // zero the state header (through RECV start) so leftover/recv_len start clean.
    ins.push(abi::move_register("%v10", abi::RET[1]));
    for o in (0..st::RECV).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, "%v10", o));
    }
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::stack_pointer(), FD),
        abi::store_u64("%v9", abi::RET[1], TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], TLS_OFFSET_CLOSED),
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
    wide_addr(symbol, abi::ARG[1], USP_NAME, &mut ins, &mut rel); // 1: pszPackage
    ins.extend([
        abi::move_immediate(abi::ARG[2], "Integer", SECPKG_CRED_OUTBOUND), // 2
        abi::move_immediate(abi::ARG[3], "Integer", "0"),                  // 3: pvLogonID=NULL
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),         // 1: phContext=NULL
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), SNAMEW), // 2: pszTargetName
        abi::move_immediate(abi::ARG[3], "Integer", ISC_REQ_FLAGS), // 3: fContextReq
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
        abi::add_immediate(abi::ARG[1], "%v10", st::RECV),
        abi::add_registers(abi::ARG[1], abi::ARG[1], "%v11"),
        abi::move_immediate(abi::ARG[2], "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::ARG[2], abi::ARG[2], "%v11"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    platform.emit_libc_call("recv", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&fail),
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), STATE),
        abi::add_immediate(abi::ARG[1], abi::ARG[1], st::CTXT), // 1: phContext=&ctxt
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), SNAMEW), // 2: pszTargetName
        abi::move_immediate(abi::ARG[3], "Integer", ISC_REQ_FLAGS), // 3: fContextReq
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
    // If SEC_E_OK → done; else (SEC_I_CONTINUE_NEEDED) reset recv_len and loop.
    ins.extend([
        abi::compare_immediate("%v15", SEC_E_OK),
        abi::branch_eq(&hs_done),
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
        abi::label(&hs_done),
    ]);

    // QueryContextAttributes(&ctxt, STREAM_SIZES, &sizes) → header/trailer/max.
    // SecPkgContext_StreamSizes { u32 cbHeader; cbTrailer; cbMaximumMessage;
    //   cBuffers; cbBlockSize } — write cbHeader/cbTrailer/cbMax into state.
    // &sizes reuses the arena SC_CRED scratch (SCHANNEL_CRED no longer needed).
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), "%v18", st::CTXT),
        abi::move_immediate(abi::ARG[1], "Integer", SECPKG_ATTR_STREAM_SIZES),
        abi::add_immediate(abi::ARG[2], "%v18", st::SC_CRED),
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
        abi::store_u64("%v9", "%v10", 16),
        abi::move_register(RESULT_VALUE_REGISTER, "%v10"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    ins.push(abi::label(&fail));
    emit_fail(symbol, ERR_NETWORK_FAILED_CODE, ERR_NETWORK_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);
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
