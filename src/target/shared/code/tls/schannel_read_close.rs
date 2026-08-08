// Included into schannel_io.rs. read (DecryptMessage) and close (shutdown).

pub(super) fn lower_tls_read(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> HelperResult {
    const REC: usize = 8;
    const MAX: usize = 16;
    const STATE: usize = 24;
    const BUFS: usize = 32; // SecBuffer[4] (64)
    const DESC: usize = 96;
    const OUTBUF: usize = 112;
    const NOUT: usize = 120;
    const COLL: usize = 128;
    const STR: usize = 136;
    const RFD: usize = 144; // socket fd, for the renegotiation handshake
    const FRAME_SIZE: usize = 0x100;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer");
    let fail = format!("{symbol}_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let enc_error = format!("{symbol}_enc");
    let done = format!("{symbol}_done");
    let have = format!("{symbol}_have");
    let dloop = format!("{symbol}_dloop");
    let dread = format!("{symbol}_dread");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        // bug-414: reject maxBytes <= 0 with ErrInvalidArgument, matching the
        // OpenSSL backend (openssl.rs). A closed resource takes precedence
        // (checked above), as it does on OpenSSL. Without this, maxBytes == 0 ran
        // a full blocking recv+DecryptMessage then served 0 bytes as OK, and a
        // negative maxBytes routed to alloc_fail/ErrOutOfMemory.
        abi::load_u64("%v9", abi::stack_pointer(), MAX),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&invalid),
        abi::load_u64("%v9", abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::store_u64("%v9", abi::stack_pointer(), STATE),
        // If undelivered plaintext remains, serve it.
        abi::load_u64("%v10", "%v9", st::LEFT_LEN),
        abi::compare_immediate("%v10", "0"),
        abi::branch_gt(&have),
    ]);
    // decrypt loop
    ins.extend([
        abi::label(&dloop),
        // If RECV empty, read more.
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v10", "%v9", st::RECV_LEN),
        abi::compare_immediate("%v10", "0"),
        abi::branch_gt(&format!("{symbol}_decrypt")),
        abi::label(&dread),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), abi::return_register(), TLS_OFFSET_FD),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v9", st::RECV_LEN),
        abi::add_immediate(abi::c_arg(1), "%v9", st::RECV),
        abi::add_registers(abi::c_arg(1), abi::c_arg(1), "%v11"),
        abi::move_immediate(abi::c_arg(2), "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::c_arg(2), abi::c_arg(2), "%v11"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_libc_call("recv", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_lt(&fail),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v9", st::RECV_LEN),
        abi::add_registers("%v11", "%v11", abi::return_register()),
        abi::store_u64("%v11", "%v9", st::RECV_LEN),
        // DecryptMessage: [0]=DATA{recv_len, RECV}, [1..3]=EMPTY
        abi::label(&format!("{symbol}_decrypt")),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v9", st::RECV_LEN),
        abi::store_u32("%v11", abi::stack_pointer(), BUFS),
        abi::move_immediate("%v12", "Integer", SECBUFFER_DATA),
        abi::store_u32("%v12", abi::stack_pointer(), BUFS + 4),
        abi::add_immediate("%v12", "%v9", st::RECV),
        abi::store_u64("%v12", abi::stack_pointer(), BUFS + 8),
    ]);
    set_secbuffer(abi::stack_pointer(), BUFS + 16, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins);
    set_secbuffer(abi::stack_pointer(), BUFS + 32, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins);
    set_secbuffer(abi::stack_pointer(), BUFS + 48, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins);
    set_secbuffer_desc(abi::stack_pointer(), DESC, "4", BUFS, &mut ins);
    // DecryptMessage(&ctxt, &desc, 0, NULL)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), DESC),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    sspi_call(symbol, "DecryptMessage", SECUR32, 4, imports, platform, &mut ins, &mut rel)?;
    // status: SEC_E_INCOMPLETE_MESSAGE → read more; <0 → fail.
    ins.push(abi::move_register("%v15", abi::return_register()));
    branch_if_incomplete("%v15", &dread, &mut ins);
    ins.extend([
        abi::compare_immediate("%v15", "0"),
        abi::branch_lt(&fail),
        // SEC_I_RENEGOTIATE (0x00090321): the peer sent post-handshake data (a TLS 1.3
        // NewSessionTicket) that must be driven back through the ISC handshake loop
        // before more application data can be decrypted.
        abi::compare_immediate("%v15", "590625"),
        abi::branch_eq(&format!("{symbol}_reneg")),
        // buffer [1] = DATA(plaintext). Copy to LEFT, set LEFT_OFF=0, LEFT_LEN=len.
        abi::load_u32("%v10", abi::stack_pointer(), BUFS + 16), // data len
        abi::load_u64("%v11", abi::stack_pointer(), BUFS + 24), // data ptr
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::add_immediate("%v6", "%v9", st::LEFT),
        abi::move_register("%v7", "%v11"), // src plaintext
    ]);
    move_bytes("%v7", "%v6", "%v10", &format!("{symbol}_ptcp"), &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v9", st::LEFT_OFF),
        abi::load_u32("%v10", abi::stack_pointer(), BUFS + 16),
        abi::store_u64("%v10", "%v9", st::LEFT_LEN),
        // Handle buffer [3] EXTRA (leftover ciphertext) → move to front of RECV.
        abi::load_u32("%v12", abi::stack_pointer(), BUFS + 48 + 4), // type of [3]
        abi::compare_immediate("%v12", SECBUFFER_EXTRA),
        abi::branch_ne(&format!("{symbol}_noextra")),
        abi::load_u32("%v13", abi::stack_pointer(), BUFS + 48), // extra len
        abi::load_u64("%v14", abi::stack_pointer(), BUFS + 48 + 8), // extra ptr
        abi::add_immediate("%v6", "%v9", st::RECV),
    ]);
    move_bytes("%v14", "%v6", "%v13", &format!("{symbol}_ex"), &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u32("%v13", abi::stack_pointer(), BUFS + 48),
        abi::store_u64("%v13", "%v9", st::RECV_LEN),
        abi::branch(&format!("{symbol}_afterextra")),
        abi::label(&format!("{symbol}_noextra")),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v9", st::RECV_LEN),
        abi::label(&format!("{symbol}_afterextra")),
        // If DecryptMessage yielded 0 plaintext bytes (renegotiation), loop.
        abi::load_u64("%v10", "%v9", st::LEFT_LEN),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&dloop),
    ]);
    // serve: n = min(LEFT_LEN, maxBytes); copy LEFT+LEFT_OFF to output.
    ins.extend([
        abi::label(&have),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v10", "%v9", st::LEFT_LEN),
        abi::load_u64("%v11", abi::stack_pointer(), MAX),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_le(&format!("{symbol}_nok")),
        abi::move_register("%v10", "%v11"),
        abi::label(&format!("{symbol}_nok")),
        abi::store_u64("%v10", abi::stack_pointer(), NOUT),
        // alloc output(n or 1)
        abi::move_register(abi::return_register(), "%v10"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), OUTBUF),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v12", "%v9", st::LEFT_OFF),
        abi::add_immediate("%v7", "%v9", st::LEFT),
        abi::add_registers("%v7", "%v7", "%v12"), // src = LEFT+off
        abi::move_register("%v6", abi::RET[1]),    // dst = output
        abi::load_u64("%v10", abi::stack_pointer(), NOUT),
    ]);
    move_bytes("%v7", "%v6", "%v10", &format!("{symbol}_serve"), &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v12", "%v9", st::LEFT_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), NOUT),
        abi::add_registers("%v12", "%v12", "%v10"),
        abi::store_u64("%v12", "%v9", st::LEFT_OFF),
        abi::load_u64("%v13", "%v9", st::LEFT_LEN),
        abi::subtract_registers("%v13", "%v13", "%v10"),
        abi::store_u64("%v13", "%v9", st::LEFT_LEN),
    ]);
    if text {
        emit_string_result_build(symbol, OUTBUF, NOUT, STR, &format!("{symbol}_scp"), &format!("{symbol}_scd"), &alloc_fail, &enc_error, &mut ins, &mut rel);
        ins.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&enc_error),
        ]);
        emit_fail(symbol, ERR_ENCODING_CODE, ERR_ENCODING_SYMBOL, &mut ins, &mut rel, &done);
    } else {
        emit_build_byte_list(symbol, &format!("{symbol}_bl"), &format!("{symbol}_bld"), OUTBUF, NOUT, Some(COLL), abi::RET[1], &alloc_fail, &mut ins, &mut rel);
        ins.push(abi::branch(&done));
    }

    // --- SEC_I_RENEGOTIATE handler: drive the buffered post-handshake data through
    // the ISC handshake loop (reusing the arena STATE scratch), then resume decrypt.
    let reneg = format!("{symbol}_reneg");
    let reneg_isc = format!("{symbol}_reneg_isc");
    let reneg_recv = format!("{symbol}_reneg_recv");
    let reneg_nosend = format!("{symbol}_reneg_nosend");
    let reneg_reset = format!("{symbol}_reneg_reset");
    ins.push(abi::label(&reneg));
    // fd → RFD; default RECV_LEN = 0 (Schannel holds the reneg data internally).
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64("%v9", "%v9", TLS_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), RFD),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v9", st::RECV_LEN),
    ]);
    // Move any DecryptMessage SECBUFFER_EXTRA ([1]/[2]/[3]) to the front of RECV.
    for buf in [BUFS + 16, BUFS + 32, BUFS + 48] {
        let next = format!("{symbol}_reneg_scan_{buf}");
        ins.extend([
            abi::load_u32("%v12", abi::stack_pointer(), buf + 4),
            abi::compare_immediate("%v12", SECBUFFER_EXTRA),
            abi::branch_ne(&next),
            abi::load_u32("%v13", abi::stack_pointer(), buf), // extra len
            abi::load_u64("%v14", abi::stack_pointer(), buf + 8), // extra ptr
            abi::load_u64("%v9", abi::stack_pointer(), STATE),
            abi::add_immediate("%v6", "%v9", st::RECV),
        ]);
        move_bytes("%v14", "%v6", "%v13", &format!("{symbol}_rex_{buf}"), &mut ins);
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE),
            abi::load_u32("%v13", abi::stack_pointer(), buf),
            abi::store_u64("%v13", "%v9", st::RECV_LEN),
            abi::branch(&reneg_isc),
            abi::label(&next),
        ]);
    }
    // reneg ISC loop: INBUF[0]={RECV_LEN, TOKEN, &RECV}, [1]=EMPTY; OUT=TOKEN(NULL).
    ins.push(abi::label(&reneg_isc));
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::RECV_LEN),
        abi::store_u32("%v11", "%v10", st::INBUF),
        abi::move_immediate("%v9", "Integer", SECBUFFER_TOKEN),
        abi::store_u32("%v9", "%v10", st::INBUF + 4),
        abi::add_immediate("%v9", "%v10", st::RECV),
        abi::store_u64("%v9", "%v10", st::INBUF + 8),
    ]);
    set_secbuffer("%v10", st::INBUF + 16, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins);
    set_secbuffer_desc("%v10", st::INDESC, "2", st::INBUF, &mut ins);
    set_secbuffer("%v10", st::OUTBUF, "0", SECBUFFER_TOKEN, abi::ZERO, &mut ins);
    set_secbuffer_desc("%v10", st::OUTDESC, "1", st::OUTBUF, &mut ins);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CRED), // 0
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(1), abi::c_arg(1), st::CTXT), // 1: phContext=&ctxt
        abi::move_immediate(abi::c_arg(2), "Integer", "0"), // 2: pszTargetName=NULL
        abi::move_immediate(abi::c_arg(3), "Integer", ISC_REQ_FLAGS), // 3
    ]);
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
    ins.push(abi::move_register("%v15", abi::return_register()));
    branch_if_incomplete("%v15", &reneg_recv, &mut ins);
    ins.extend([
        abi::compare_immediate("%v15", "0"),
        abi::branch_lt(&fail),
    ]);
    emit_send_token(symbol, RFD, STATE, st::OUTBUF, &reneg_nosend, "rtok", &fail, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&reneg_nosend));
    ins.extend([
        // SEC_E_OK → renegotiation complete, decrypt the application data.
        abi::compare_immediate("%v15", "0"),
        abi::branch_eq(&dloop),
        // else CONTINUE: consume INBUF[1] EXTRA (move to front of RECV) or recv more.
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::INBUF + 16 + 4), // type of buf[1]
        abi::compare_immediate("%v9", SECBUFFER_EXTRA),
        abi::branch_ne(&reneg_reset),
        abi::load_u32("%v11", "%v10", st::INBUF + 16), // extra len
        abi::load_u32("%v12", "%v10", st::RECV_LEN),
        abi::subtract_registers("%v13", "%v12", "%v11"), // src offset
        abi::add_immediate("%v14", "%v10", st::RECV),
        abi::add_registers("%v14", "%v14", "%v13"),
        abi::add_immediate("%v6", "%v10", st::RECV),
    ]);
    move_bytes("%v14", "%v6", "%v11", &format!("{symbol}_rgex"), &mut ins);
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::branch(&reneg_isc),
        // CONTINUE with no buffered handshake bytes left: the post-handshake exchange
        // is drained — resume the main decrypt loop, whose fresh recv reads the
        // application data (do NOT recv more expecting handshake data that won't come).
        abi::label(&reneg_reset),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
        abi::branch(&dloop),
        // reneg_recv: SEC_E_INCOMPLETE_MESSAGE needs more bytes of the current record.
        abi::label(&reneg_recv),
        // recv(fd, RECV+RECV_LEN, RECV_CAP-RECV_LEN, 0)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RFD),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::add_immediate(abi::c_arg(1), "%v10", st::RECV),
        abi::add_registers(abi::c_arg(1), abi::c_arg(1), "%v11"),
        abi::move_immediate(abi::c_arg(2), "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::c_arg(2), abi::c_arg(2), "%v11"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_libc_call("recv", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&fail),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::add_registers("%v11", "%v11", abi::return_register()),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::branch(&reneg_isc),
    ]);

    ins.push(abi::label(&peer_closed));
    emit_fail(symbol, ERR_CONNECTION_CLOSED_CODE, ERR_CONNECTION_CLOSED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, ERR_RESOURCE_CLOSED_CODE, ERR_RESOURCE_CLOSED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&fail));
    emit_fail(symbol, ERR_NETWORK_FAILED_CODE, ERR_NETWORK_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}

pub(super) fn lower_tls_close(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const REC: usize = 8;
    const STATE: usize = 16;
    const FD: usize = 24;
    const SHUT: usize = 32; // DWORD SCHANNEL_SHUTDOWN
    const BUFS: usize = 48; // SecBuffer[1] (ApplyControlToken input, stack)
    const DESC: usize = 64;
    const SRV: usize = 72; // cached st::SERVER marker (server-accepted socket)
    // The close_notify ISC's SecBuffer/desc/attrs/expiry live in the arena STATE
    // (st::OUTBUF/OUTDESC/ATTRS/EXPIRY) so their pointers survive sspi_call_ext's
    // sub_sp (see there); the 2-arg ApplyControlToken keeps its stack input.
    const FRAME_SIZE: usize = 0x100;

    let already = format!("{symbol}_already");
    let done = format!("{symbol}_done");
    let no_tok = format!("{symbol}_notok");
    let skip_free = format!("{symbol}_skip_free");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v9", abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::store_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v10", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("%v10", abi::stack_pointer(), FD),
        // A server-accepted socket (st::SERVER) shares the listener's credential
        // and has a server-side context; it must NOT generate a client
        // close_notify via ISC, nor free the shared credential. Skip straight to
        // DeleteSecurityContext + closesocket.
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v9", st::SERVER),
        abi::store_u64("%v9", abi::stack_pointer(), SRV),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&no_tok),
        // ApplyControlToken with SCHANNEL_SHUTDOWN(1).
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u32("%v9", abi::stack_pointer(), SHUT),
        abi::move_immediate("%v9", "Integer", "4"),
        abi::store_u32("%v9", abi::stack_pointer(), BUFS),
        abi::move_immediate("%v9", "Integer", SECBUFFER_TOKEN),
        abi::store_u32("%v9", abi::stack_pointer(), BUFS + 4),
        abi::add_immediate("%v9", abi::stack_pointer(), SHUT),
        abi::store_u64("%v9", abi::stack_pointer(), BUFS + 8),
    ]);
    set_secbuffer_desc(abi::stack_pointer(), DESC, "1", BUFS, &mut ins);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), DESC),
    ]);
    sspi_call(symbol, "ApplyControlToken", SECUR32, 2, imports, platform, &mut ins, &mut rel)?;
    // ISC to produce the close_notify alert. SecBuffer/desc in the arena.
    ins.push(abi::load_u64("%v18", abi::stack_pointer(), STATE));
    set_secbuffer("%v18", st::OUTBUF, "0", SECBUFFER_TOKEN, abi::ZERO, &mut ins);
    set_secbuffer_desc("%v18", st::OUTDESC, "1", st::OUTBUF, &mut ins);
    // ISC(&cred, &ctxt, NULL, flags, 0, 0, NULL, 0, &ctxt, &outdesc, &attrs, &expiry)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CRED), // 0: phCredential
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(1), abi::c_arg(1), st::CTXT), // 1: phContext=&ctxt
        abi::move_immediate(abi::c_arg(2), "Integer", "0"), // 2: pszTargetName=NULL
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
    // Best-effort send of the alert; ignore errors.
    ins.extend([
        abi::load_u64("%v5", abi::stack_pointer(), STATE),
        abi::load_u32("%v8", "%v5", st::OUTBUF),
        abi::compare_immediate("%v8", "0"),
        abi::branch_eq(&no_tok),
        abi::load_u64("%v9", "%v5", st::OUTBUF + 8),
    ]);
    send_all(symbol, FD, "%v9", "%v8", "shut", &no_tok, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::load_u64("%v5", abi::stack_pointer(), STATE));
    ins.push(abi::load_u64(abi::return_register(), "%v5", st::OUTBUF + 8));
    sspi_call(symbol, "FreeContextBuffer", SECUR32, 1, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&no_tok));
    // DeleteSecurityContext, FreeCredentialsHandle, closesocket.
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
    ]);
    sspi_call(symbol, "DeleteSecurityContext", SECUR32, 1, imports, platform, &mut ins, &mut rel)?;
    // FreeCredentialsHandle only for a client-owned credential; a server-accepted
    // socket shares the listener's, freed once at closeListener.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SRV),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&skip_free),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CRED),
    ]);
    sspi_call(symbol, "FreeCredentialsHandle", SECUR32, 1, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&skip_free));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // Mark closed.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", TLS_OFFSET_CLOSED),
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}

// plan-76-B: tls::poll(sock[, timeoutMs]) AS Boolean on schannel.
// readable = STATE[LEFT_LEN] > 0 (undelivered decrypted plaintext already buffered
// from a prior DecryptMessage) OR WSAPoll(fd, POLLRDNORM) indicates the socket is
// readable. The buffered fast-path is mandatory: a DecryptMessage can leave plaintext
// in the carry-over buffer with the socket idle, which an fd-only poll would miss.
// x0 = sock record, x1 = timeoutMs.
pub(super) fn lower_tls_poll(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const TIMEOUT: usize = 8;
    const POLLFD: usize = 16; // WSAPOLLFD { SOCKET fd; SHORT events; SHORT revents } (16 bytes)
    const FRAME_SIZE: usize = 48;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let poll_fail = format!("{symbol}_poll_fail");
    let poll_infinite = format!("{symbol}_poll_infinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        // Buffered decrypted plaintext? STATE ptr is at record[16] (schannel repurposes
        // the SSL slot; the read helper loads it from the same literal offset);
        // LEFT_LEN is the undelivered plaintext byte count.
        abi::load_u64("%v9", abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::load_u64("%v10", "%v9", st::LEFT_LEN),
        abi::compare_immediate("%v10", "0"),
        abi::branch_gt(&ready),
        // Normalize the timeout (net::poll policy): sentinel→-1 (block), <0→invalid,
        // >0→clamp INT_MAX. No external call precedes WSAPoll, so the record pointer in
        // x0 stays live for the fd load below.
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::move_immediate("%v10", "Integer", super::super::TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&poll_infinite),
        abi::compare_immediate("%v9", "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate("%v10", "Integer", "2147483647"),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_le(&timeout_ok),
        abi::move_register("%v9", "%v10"),
        abi::branch(&timeout_ok),
        abi::label(&poll_infinite),
        abi::bitwise_not("%v9", abi::ZERO),
        abi::label(&timeout_ok),
        abi::store_u64("%v9", abi::stack_pointer(), TIMEOUT),
        // WSAPOLLFD { fd; events = POLLRDNORM; revents = 0 }
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD),
        abi::move_immediate("%v10", "Integer", POLLRDNORM),
        abi::store_u16("%v10", abi::stack_pointer(), POLLFD + 8),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), POLLFD + 10),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT),
    ]);
    platform.emit_libc_call("WSAPoll", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        // WSAPoll returns a C int; sign-extend before the signed compares.
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
        // WSAPoll has no EINTR (no POSIX signals); a negative return is a hard error.
        abi::label(&poll_fail),
    ]);
    emit_fail(symbol, ERR_NETWORK_FAILED_CODE, ERR_NETWORK_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, ERR_RESOURCE_CLOSED_CODE, ERR_RESOURCE_CLOSED_SYMBOL, &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}
