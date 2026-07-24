// Included into schannel_impl.rs. Handshake support helpers + read/write/close.

/// Marshal an MFB `String` (pointer at `str_off`) into a fresh arena UTF-16
/// NUL-terminated buffer; store the buffer pointer at `out_off`. Uses
/// MultiByteToWideChar(CP_UTF8). Branches to `fail` on allocation failure.
fn emit_wide_cstring(
    symbol: &str,
    str_off: usize,
    out_off: usize,
    fail: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    // Allocate 64 KiB (32767 wchars, Windows max) — the serverName is short.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::ARG[1], "Integer", "2"),
    ]);
    emit_alloc(symbol, ins, rel, fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), out_off));
    // MultiByteToWideChar(CP_UTF8=65001, 0, str+8, -1, wbuf, 32768). The MFB
    // String is [u64 len][utf8 bytes][... ]; the data starts at +8 and is NUL-
    // terminated by the builder, so cbMultiByte = -1 works.
    const FRAME: usize = 0x30;
    ins.extend([
        abi::subtract_stack(FRAME),
        abi::move_immediate(abi::return_register(), "Integer", "65001"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), str_off + FRAME),
        abi::add_immediate(abi::ARG[2], abi::ARG[2], 8),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::subtract_immediate(abi::ARG[3], abi::ARG[3], 1), // cbMultiByte = -1
        abi::load_u64("%v9", abi::stack_pointer(), out_off + FRAME),
        abi::store_u64("%v9", abi::stack_pointer(), 0x20), // lpWideCharStr (5th)
        abi::move_immediate("%v9", "Integer", "32768"),
        abi::store_u64("%v9", abi::stack_pointer(), 0x28), // cchWideChar (6th)
    ]);
    ins.push(abi::branch_link("MultiByteToWideChar"));
    rel.push(CodeRelocation {
        from: symbol.to_string(),
        to: "MultiByteToWideChar".to_string(),
        kind: RelocIntent::Call,
        binding: "external".to_string(),
        library: Some("kernel32.dll".to_string()),
    });
    ins.push(abi::add_stack(FRAME));
}

/// If the SecBuffer at `buf_off` holds a token (cbBuffer > 0), send its bytes to
/// the socket and FreeContextBuffer(pvBuffer). Branch to `skip` when empty, `fail`
/// on send error.
#[allow(clippy::too_many_arguments)]
/// Send the SecBuffer token at arena `state.buf_off` (SecBuffer{cbBuffer, _, pvBuffer})
/// over the socket, then FreeContextBuffer it. `state_off` is the frame slot of the
/// arena STATE pointer; `buf_off` is the SecBuffer's offset within STATE.
#[allow(clippy::too_many_arguments)]
fn emit_send_token(
    symbol: &str,
    fd_off: usize,
    state_off: usize,
    buf_off: usize,
    skip: &str,
    tag: &str,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    ins.extend([
        abi::load_u64("%v5", abi::stack_pointer(), state_off), // STATE
        abi::load_u32("%v8", "%v5", buf_off),                  // cbBuffer
        abi::compare_immediate("%v8", "0"),
        abi::branch_eq(skip),
        abi::load_u64("%v9", "%v5", buf_off + 8), // pvBuffer
    ]);
    send_all(symbol, fd_off, "%v9", "%v8", tag, fail, imports, platform, ins, rel)?;
    // FreeContextBuffer(pvBuffer)
    ins.push(abi::load_u64("%v5", abi::stack_pointer(), state_off));
    ins.push(abi::load_u64(abi::return_register(), "%v5", buf_off + 8));
    sspi_call(symbol, "FreeContextBuffer", SECUR32, 1, imports, platform, ins, rel)?;
    Ok(())
}

/// Enforce the peer certificate's HOSTNAME against `serverName` (wide, at
/// `snamew_off`) using the SSL chain policy. Branches to `fail` on any mismatch or
/// error. This is the check Schannel does NOT do automatically (plan §correctness).
#[allow(clippy::too_many_arguments)]
fn emit_verify_hostname(
    symbol: &str,
    state_off: usize,
    snamew_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Scratch structs are carved in a self-contained frame:
    //   [0x20 .. 0x28)  certContext ptr (out)
    //   [0x28 .. 0x30)  chainContext ptr (out)
    //   [0x30 .. 0x60)  CERT_CHAIN_PARA (zeroed; cbSize=0x0c... use sizeof)
    //   [0x60 .. 0xB0)  SSL_EXTRA_CERT_CHAIN_POLICY_PARA + CERT_CHAIN_POLICY_PARA
    //   [0xB0 .. 0xC0)  CERT_CHAIN_POLICY_STATUS (out)
    const FRAME: usize = 0x100;
    let ok = format!("{symbol}_hn_ok");
    ins.push(abi::subtract_stack(FRAME));
    // QueryContextAttributes(&ctxt, REMOTE_CERT_CONTEXT, &certCtx)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), state_off + FRAME),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
        abi::move_immediate(abi::ARG[1], "Integer", SECPKG_ATTR_REMOTE_CERT_CONTEXT),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), 0x20),
    ]);
    sspi_call(symbol, "QueryContextAttributesW", SECUR32, 3, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(fail));
    // Zero CERT_CHAIN_PARA (0x30 bytes) and set cbSize.
    for o in (0x30..0x60).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), o));
    }
    ins.extend([
        abi::move_immediate("%v9", "Integer", "12"), // sizeof(CERT_CHAIN_PARA) minimal
        abi::store_u32("%v9", abi::stack_pointer(), 0x30),
    ]);
    // CertGetCertificateChain(NULL, certCtx, NULL, NULL, &chainPara, 0, NULL, &chainCtx)
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), 0x20),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::add_immediate(abi::ARG[4], abi::stack_pointer(), 0x30),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
        abi::add_immediate(abi::ARG[7], abi::stack_pointer(), 0x28),
    ]);
    sspi_call(symbol, "CertGetCertificateChain", CRYPT32, 8, imports, platform, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail), // BOOL FALSE
    ]);
    // Build SSL_EXTRA_CERT_CHAIN_POLICY_PARA { cbSize; dwAuthType=SERVER(1);
    //   fdwChecks=0; pwszServerName } at 0x60; CERT_CHAIN_POLICY_PARA { cbSize;
    //   dwFlags=0; pvExtraPolicyPara=&ssl } at 0x80; status at 0xB0.
    for o in (0x60..0xC0).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), o));
    }
    ins.extend([
        abi::move_immediate("%v9", "Integer", "24"), // sizeof SSL_EXTRA...PARA (x64)
        abi::store_u32("%v9", abi::stack_pointer(), 0x60),
        abi::move_immediate("%v9", "Integer", "1"), // AUTHTYPE_SERVER
        abi::store_u32("%v9", abi::stack_pointer(), 0x64),
        abi::load_u64("%v9", abi::stack_pointer(), snamew_off + FRAME),
        abi::store_u64("%v9", abi::stack_pointer(), 0x70), // pwszServerName
        abi::move_immediate("%v9", "Integer", "16"), // sizeof CERT_CHAIN_POLICY_PARA
        abi::store_u32("%v9", abi::stack_pointer(), 0x80),
        abi::add_immediate("%v9", abi::stack_pointer(), 0x60),
        abi::store_u64("%v9", abi::stack_pointer(), 0x88), // pvExtraPolicyPara
        abi::move_immediate("%v9", "Integer", "16"), // status cbSize
        abi::store_u32("%v9", abi::stack_pointer(), 0xB0),
    ]);
    // CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, chainCtx, &para, &status)
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", CERT_CHAIN_POLICY_SSL),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), 0x28),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), 0x80),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), 0xB0),
    ]);
    sspi_call(symbol, "CertVerifyCertificateChainPolicy", CRYPT32, 4, imports, platform, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail), // BOOL FALSE = call failed
        // status.dwError (at 0xB0+8) must be 0.
        abi::load_u32("%v9", abi::stack_pointer(), 0xB0 + 8),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&ok),
        abi::add_stack(FRAME),
        abi::branch(fail),
        abi::label(&ok),
        abi::add_stack(FRAME),
    ]);
    Ok(())
}

// ---------------------------------------------------------------------------
// write: EncryptMessage each chunk, send [header][data][trailer].
// ---------------------------------------------------------------------------
pub(super) fn lower_tls_write(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> HelperResult {
    const REC: usize = 8;
    const SRC: usize = 16;
    const REMAIN: usize = 24;
    const STATE: usize = 32;
    const SENDBUF: usize = 40; // arena scratch: header+data+trailer
    const CHUNK: usize = 48;
    const FD: usize = 56;
    const BUFS: usize = 64; // SecBuffer[4] (64)
    const DESC: usize = 128;
    const FRAME_SIZE: usize = 0x100;

    let closed = format!("{symbol}_closed");
    let fail = format!("{symbol}_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let wloop = format!("{symbol}_wloop");
    let wdone = format!("{symbol}_wdone");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // return_register = resource; ARG[1] = data (String/List).
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), 16),
        abi::store_u64("%v9", abi::stack_pointer(), STATE),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD),
        // data pointer + length: String/List OF Byte both carry [u64 len][bytes].
        abi::add_immediate("%v10", abi::ARG[1], 8),
        abi::store_u64("%v10", abi::stack_pointer(), SRC),
        abi::load_u64("%v10", abi::ARG[1], 0),
        abi::store_u64("%v10", abi::stack_pointer(), REMAIN),
    ]);
    let _ = text;
    // Allocate a send buffer sized header + maxmsg + trailer.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::HEADER),
        abi::load_u32("%v12", "%v10", st::MAXMSG),
        abi::load_u32("%v13", "%v10", st::TRAILER),
        abi::add_registers(abi::return_register(), "%v11", "%v12"),
        abi::add_registers(abi::return_register(), abi::return_register(), "%v13"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), SENDBUF));

    ins.extend([
        abi::label(&wloop),
        abi::load_u64("%v10", abi::stack_pointer(), REMAIN),
        abi::compare_immediate("%v10", "0"),
        abi::branch_le(&wdone),
        // chunk = min(remaining, maxmsg)
        abi::load_u64("%v11", abi::stack_pointer(), STATE),
        abi::load_u32("%v12", "%v11", st::MAXMSG),
        abi::compare_registers("%v10", "%v12"),
        abi::branch_le(&format!("{symbol}_ck")),
        abi::move_register("%v10", "%v12"),
        abi::label(&format!("{symbol}_ck")),
        abi::store_u64("%v10", abi::stack_pointer(), CHUNK),
        // copy plaintext to SENDBUF + header
        abi::load_u64("%v13", abi::stack_pointer(), STATE),
        abi::load_u32("%v14", "%v13", st::HEADER),
        abi::load_u64("%v6", abi::stack_pointer(), SENDBUF),
        abi::add_registers("%v6", "%v6", "%v14"), // dst = sendbuf+header
        abi::load_u64("%v7", abi::stack_pointer(), SRC),
    ]);
    move_bytes("%v7", "%v6", "%v10", &format!("{symbol}_cpin"), &mut ins);
    // SecBuffers: [0]=HEADER{header, sendbuf}, [1]=DATA{chunk, sendbuf+header},
    //   [2]=TRAILER{trailer, sendbuf+header+chunk}, [3]=EMPTY.
    ins.extend([
        abi::load_u64("%v13", abi::stack_pointer(), STATE),
        abi::load_u32("%v14", "%v13", st::HEADER),
        abi::load_u64("%v6", abi::stack_pointer(), SENDBUF),
        abi::store_u32("%v14", abi::stack_pointer(), BUFS),
        abi::move_immediate("%v9", "Integer", SECBUFFER_STREAM_HEADER),
        abi::store_u32("%v9", abi::stack_pointer(), BUFS + 4),
        abi::store_u64("%v6", abi::stack_pointer(), BUFS + 8),
        // [1] DATA
        abi::load_u64("%v10", abi::stack_pointer(), CHUNK),
        abi::store_u32("%v10", abi::stack_pointer(), BUFS + 16),
        abi::move_immediate("%v9", "Integer", SECBUFFER_DATA),
        abi::store_u32("%v9", abi::stack_pointer(), BUFS + 20),
        abi::add_registers("%v7", "%v6", "%v14"),
        abi::store_u64("%v7", abi::stack_pointer(), BUFS + 24),
        // [2] TRAILER
        abi::load_u32("%v11", "%v13", st::TRAILER),
        abi::store_u32("%v11", abi::stack_pointer(), BUFS + 32),
        abi::move_immediate("%v9", "Integer", SECBUFFER_STREAM_TRAILER),
        abi::store_u32("%v9", abi::stack_pointer(), BUFS + 36),
        abi::add_registers("%v7", "%v7", "%v10"),
        abi::store_u64("%v7", abi::stack_pointer(), BUFS + 40),
    ]);
    set_secbuffer(abi::stack_pointer(), BUFS + 48, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins);
    set_secbuffer_desc(abi::stack_pointer(), DESC, "4", BUFS, &mut ins);
    // EncryptMessage(&ctxt, 0, &desc, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), DESC),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    sspi_call(symbol, "EncryptMessage", SECUR32, 4, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));
    // send header+data+trailer = sum of the three cbBuffer.
    ins.extend([
        abi::load_u32("%v6", abi::stack_pointer(), BUFS),
        abi::load_u32("%v7", abi::stack_pointer(), BUFS + 16),
        abi::add_registers("%v6", "%v6", "%v7"),
        abi::load_u32("%v7", abi::stack_pointer(), BUFS + 32),
        abi::add_registers("%v6", "%v6", "%v7"), // total len
        abi::load_u64("%v7", abi::stack_pointer(), SENDBUF), // buf
    ]);
    send_all(symbol, FD, "%v7", "%v6", "enc", &fail, imports, platform, &mut ins, &mut rel)?;
    // advance src/remaining
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), CHUNK),
        abi::load_u64("%v11", abi::stack_pointer(), SRC),
        abi::add_registers("%v11", "%v11", "%v10"),
        abi::store_u64("%v11", abi::stack_pointer(), SRC),
        abi::load_u64("%v11", abi::stack_pointer(), REMAIN),
        abi::subtract_registers("%v11", "%v11", "%v10"),
        abi::store_u64("%v11", abi::stack_pointer(), REMAIN),
        abi::branch(&wloop),
        abi::label(&wdone),
        // result = original length
        abi::load_u64(RESULT_VALUE_REGISTER, abi::ARG[1], 0),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    ins.push(abi::label(&closed));
    emit_fail(symbol, ERR_RESOURCE_CLOSED_CODE, ERR_RESOURCE_CLOSED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&fail));
    emit_fail(symbol, ERR_NETWORK_FAILED_CODE, ERR_NETWORK_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}

include!("schannel_read_close.rs");
