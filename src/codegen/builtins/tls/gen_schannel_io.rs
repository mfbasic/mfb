// Included into schannel_impl.rs. Handshake support helpers + read/write/close.

/// Marshal an MFB `String` (pointer at `str_off`) into a fresh arena UTF-16
/// NUL-terminated buffer; store the buffer pointer at `out_off`. Uses
/// MultiByteToWideChar(CP_UTF8). Branches to `fail` on allocation failure.
fn emit_wide_cstring(
    symbol: &str,
    str_off: usize,
    out_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Allocate 64 KiB (32767 wchars, Windows max) — the serverName is short.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::c_arg(1), "Integer", "2"),
    ]);
    emit_alloc(symbol, ins, rel, fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), out_off));
    // MultiByteToWideChar(CP_UTF8=65001, 0, str+8, -1, wbuf, 32768). The MFB String
    // is [u64 len][utf8 bytes]; the data starts at +8 and is NUL-terminated by the
    // builder, so cbMultiByte = -1 works. All six args set in ARG roles at DEPTH 0;
    // sspi_call spills args 4/5 (wbuf, cchWideChar) from those remap-managed roles
    // — NOT from a plain vreg carried across the sub_sp, which could spill and reload
    // body_shift off (a garbage lpWideCharStr scribbled over the socket fd slot).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "65001"), // 0: CP_UTF8
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),               // 1: dwFlags
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), str_off),
        abi::add_immediate(abi::c_arg(2), abi::c_arg(2), 8),                 // 2: lpMultiByteStr
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::subtract_immediate(abi::c_arg(3), abi::c_arg(3), 1),           // 3: cbMultiByte=-1
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), out_off),       // 4: lpWideCharStr
        abi::move_immediate(abi::c_arg(5), "Integer", "32768"),           // 5: cchWideChar
    ]);
    sspi_call(symbol, "MultiByteToWideChar", "kernel32.dll", 6, imports, platform, ins, rel)?;
    Ok(())
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
 vregs: &mut Vregs) -> Result<(), String> {
    let v5 = vregs.next();
    let v8 = vregs.next();
    let v9 = vregs.next();
    ins.extend([
        abi::load_u64(&v5, abi::stack_pointer(), state_off), // STATE
        abi::load_u32(&v8, &v5, buf_off),                  // cbBuffer
        abi::compare_immediate(&v8, "0"),
        abi::branch_eq(skip),
        abi::load_u64(&v9, &v5, buf_off + 8), // pvBuffer
    ]);
    send_all(symbol, fd_off, &v9, &v8, tag, fail, imports, platform, ins, rel, vregs)?;
    // FreeContextBuffer(pvBuffer)
    ins.push(abi::load_u64(&v5, abi::stack_pointer(), state_off));
    ins.push(abi::load_u64(abi::return_register(), &v5, buf_off + 8));
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
    // bug-477 `allowSelfSigned`: frame slot holding the flag (0/1).
    allow_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
 vregs: &mut Vregs) -> Result<(), String> {
    let v8 = vregs.next();
    let v9 = vregs.next();
    // Scratch structs live in the ARENA at STATE + st::SC_CRED (the SCHANNEL_CRED /
    // SecBuffer area is idle post-handshake), so no manual sub_sp is needed and every
    // pointer is absolute — a `[sp+off+FRAME]` read at DEPTH 1 would be body_shift off
    // (finalize_frame only shifts depth-0 accesses), which corrupted a return address
    // into RIP=0. `%v8` = STATE+SC_CRED, reloaded before each block because the nested
    // sspi_call clobbers it (it is only ever set/used at DEPTH 0).
    const CERTCTX: usize = 0; // cert context ptr (out)
    const CHAINCTX: usize = 8; // chain context ptr (out)
    const CHAINPARA: usize = 0x10; // CERT_CHAIN_PARA (0x30)
    const SSLPARA: usize = 0x40; // SSL_EXTRA_CERT_CHAIN_POLICY_PARA (0x18)
    const POLICYPARA: usize = 0x60; // CERT_CHAIN_POLICY_PARA (0x10)
    const STATUS: usize = 0x90; // CERT_CHAIN_POLICY_STATUS (0x10)
    let load_base = |ins: &mut Vec<CodeInstruction>| {
        ins.push(abi::load_u64(&v8, abi::stack_pointer(), state_off));
        ins.push(abi::add_immediate(&v8, &v8, st::SC_CRED));
    };
    // QueryContextAttributes(&ctxt, REMOTE_CERT_CONTEXT, &certCtx)
    load_base(ins);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), state_off),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
        abi::move_immediate(abi::c_arg(1), "Integer", SECPKG_ATTR_REMOTE_CERT_CONTEXT),
        abi::add_immediate(abi::c_arg(2), &v8, CERTCTX),
    ]);
    sspi_call(symbol, "QueryContextAttributesW", SECUR32, 3, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(fail));
    // Zero CERT_CHAIN_PARA (0x30 bytes) and set cbSize.
    load_base(ins);
    for o in (0..0x30).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, &v8, CHAINPARA + o));
    }
    ins.extend([
        abi::move_immediate(&v9, "Integer", "12"), // sizeof(CERT_CHAIN_PARA) minimal
        abi::store_u32(&v9, &v8, CHAINPARA),
        // CertGetCertificateChain(NULL, certCtx, NULL, NULL, &chainPara, 0, NULL, &chainCtx)
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::c_arg(1), &v8, CERTCTX),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::add_immediate(abi::c_arg(4), &v8, CHAINPARA),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
        abi::add_immediate(abi::c_arg(7), &v8, CHAINCTX),
    ]);
    sspi_call(symbol, "CertGetCertificateChain", CRYPT32, 8, imports, platform, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail), // BOOL FALSE
    ]);
    // SSL_EXTRA_CERT_CHAIN_POLICY_PARA { cbSize; dwAuthType=SERVER(1); fdwChecks=0;
    //   pwszServerName } at SSLPARA; CERT_CHAIN_POLICY_PARA { cbSize; dwFlags=0;
    //   pvExtraPolicyPara=&ssl } at POLICYPARA; status at STATUS.
    load_base(ins);
    for o in (0..0x60).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, &v8, SSLPARA + o));
    }
    ins.extend([
        abi::move_immediate(&v9, "Integer", "24"), // sizeof SSL_EXTRA...PARA (x64)
        abi::store_u32(&v9, &v8, SSLPARA),
        abi::move_immediate(&v9, "Integer", "1"), // AUTHTYPE_SERVER
        abi::store_u32(&v9, &v8, SSLPARA + 4),
        abi::load_u64(&v9, abi::stack_pointer(), snamew_off),
        abi::store_u64(&v9, &v8, SSLPARA + 16), // pwszServerName (x64 offset 16, after fdwChecks@8 + 4B pad@12)
        abi::move_immediate(&v9, "Integer", "16"), // sizeof CERT_CHAIN_POLICY_PARA
        abi::store_u32(&v9, &v8, POLICYPARA),
        abi::add_immediate(&v9, &v8, SSLPARA),
        abi::store_u64(&v9, &v8, POLICYPARA + 8), // pvExtraPolicyPara
        abi::move_immediate(&v9, "Integer", "16"), // status cbSize
        abi::store_u32(&v9, &v8, STATUS),
    ]);
    {
        // bug-477: SSL_EXTRA_CERT_CHAIN_POLICY_PARA::fdwChecks is at SSLPARA + 8
        // and the blanket zeroing loop above leaves it 0 (the strict default).
        // With `allowSelfSigned` set, put IGNORE_UNKNOWN_CA there and nothing
        // else, so the SSL policy forgives an untrusted root while
        // `pwszServerName` still drives the name match and CERT_E_EXPIRED still
        // surfaces in dwError.
        let strict = format!("{symbol}_policy_strict");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), allow_off),
            abi::compare_immediate(&v9, "0"),
            abi::branch_eq(&strict),
            abi::move_immediate(&v9, "Integer", SECURITY_FLAG_IGNORE_UNKNOWN_CA),
            abi::store_u32(&v9, &v8, SSLPARA + 8),
            abi::label(&strict),
        ]);
    }
    load_base(ins);
    ins.extend([
        // CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, chainCtx, &para, &status)
        abi::move_immediate(abi::return_register(), "Integer", CERT_CHAIN_POLICY_SSL),
        abi::load_u64(abi::c_arg(1), &v8, CHAINCTX),
        abi::add_immediate(abi::c_arg(2), &v8, POLICYPARA),
        abi::add_immediate(abi::c_arg(3), &v8, STATUS),
    ]);
    sspi_call(symbol, "CertVerifyCertificateChainPolicy", CRYPT32, 4, imports, platform, ins, rel)?;
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_eq(fail)); // BOOL FALSE = call failed
    // CERT_CHAIN_POLICY_STATUS { cbSize@0; dwError@4; lChainIndex@8; ... }.
    // dwError (at STATUS+4) must be 0 — NOT +8 (lChainIndex, which is -1 on success).
    load_base(ins);
    ins.extend([
        abi::load_u32(&v9, &v8, STATUS + 4),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(fail),
    ]);
    Ok(())
}

#[cfg(test)]
mod verify_hostname_tests {
    use super::*;
    use crate::codegen::engine::tests::TestPlatform;
    use crate::arch::ops::CodeOp;
    use std::collections::HashMap;

    // bug-477: `allowSelfSigned` must relax the chain policy by EXACTLY one bit.
    //
    // The tempting Schannel shortcut is to widen CERT_CHAIN_POLICY_PARA::dwFlags
    // to the whole CERT_CHAIN_POLICY_IGNORE_* family, or to drop the dwError == 0
    // requirement. Either silently stops the name and expiry checks — the exact
    // MITM hazard bug-477 forbids — and neither shows up in a positive
    // "self-signed is accepted" fixture. This pins the emitted constant.
    #[test]
    fn relaxation_sets_only_the_unknown_ca_flag() {
        const SSLPARA: usize = 0x40; // must match emit_verify_hostname's local const
        let imports = HashMap::new();
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        let mut vregs = Vregs::new();
        emit_verify_hostname(
            "t_vh", 8, 200, 208, "t_vh_fail", &imports, &TestPlatform,
            &mut ins, &mut rel, &mut vregs,
        )
        .expect("lower emit_verify_hostname");

        // Every immediate written into dwFlags (POLICYPARA + 4).
        let written: Vec<String> = ins
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                i.op == CodeOp::StrU32
                    && i.get("offset").as_deref() == Some((SSLPARA + 8).to_string().as_str())
            })
            .filter_map(|(idx, i)| {
                let src = i.get("src")?;
                ins[..idx].iter().rev().find_map(|prev| {
                    (prev.op == CodeOp::MovImm && prev.get("dst")? == src)
                        .then(|| prev.get("value"))?
                })
            })
            .collect();
        assert_eq!(
            written,
            vec![SECURITY_FLAG_IGNORE_UNKNOWN_CA.to_string()],
            "bug-477: fdwChecks must receive IGNORE_UNKNOWN_CA and nothing else — any \
             other SECURITY_FLAG_IGNORE_* bit would stop the name or expiry \
             check that `allowSelfSigned` is required to preserve"
        );

        // The post-policy `dwError == 0` requirement must survive: relaxing that
        // instead of dwFlags would accept EVERY policy failure.
        assert!(
            ins.iter().any(|i| i.op == CodeOp::BranchNe
                && i.get("target").as_deref() == Some("t_vh_fail")),
            "bug-477: the dwError != 0 -> fail branch must still be emitted"
        );
    }

    // bug-413: the wide server-name pointer must be stored into
    // SSL_EXTRA_CERT_CHAIN_POLICY_PARA::pwszServerName, which on Win64 (with the
    // declared `cbSize = 24` four-field x64 layout: cbSize@0, dwAuthType@4,
    // fdwChecks@8, 4-byte pad@12, pwszServerName@16) is at SSLPARA + 16. Storing
    // it at SSLPARA + 8 lands in fdwChecks (+ pad) and leaves pwszServerName NULL,
    // so CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL) performs NO
    // hostname match — any chain-trusted cert is accepted for any hostname (MITM).
    #[test]
    fn server_name_pointer_stored_at_pwsz_server_name_offset() {
        const SSLPARA: usize = 0x40; // must match emit_verify_hostname's local const
        const SNAMEW_OFF: usize = 200; // distinctive frame slot for the wide name ptr
        const ALLOW_OFF: usize = 208; // bug-477 allowSelfSigned slot
        let imports = HashMap::new();
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        let mut vregs = Vregs::new();
        emit_verify_hostname(
            "t_vh",
            8,
            SNAMEW_OFF,
            ALLOW_OFF,
            "t_vh_fail",
            &imports,
            &TestPlatform,
            &mut ins,
            &mut rel,
            &mut vregs,
        )
        .expect("lower emit_verify_hostname");

        // Locate the load of the wide server-name pointer from [sp + SNAMEW_OFF].
        let load_idx = ins
            .iter()
            .position(|i| {
                i.op == CodeOp::LdrU64
                    && i.get("base").as_deref() == Some(abi::stack_pointer())
                    && i.get("offset").as_deref() == Some(SNAMEW_OFF.to_string().as_str())
            })
            .expect("emit_verify_hostname must load the wide server-name pointer");
        let name_reg = ins[load_idx]
            .get("dst")
            .expect("load has a dst register")
            .to_string();

        // The next store of that register into the SSLPARA struct is the
        // pwszServerName write; it must target SSLPARA + 16, never SSLPARA + 8.
        let store = ins[load_idx + 1..]
            .iter()
            .find(|i| i.op == CodeOp::StrU64 && i.get("src").as_deref() == Some(name_reg.as_str()))
            .expect("the loaded server-name pointer must be stored into the policy para");
        assert_eq!(
            store.get("offset").as_deref(),
            Some((SSLPARA + 16).to_string().as_str()),
            "pwszServerName must be written at SSLPARA + 16 (offset {}); \
             storing it at SSLPARA + 8 (offset {}) leaves pwszServerName NULL and \
             disables Schannel hostname verification (bug-413 TLS MITM)",
            SSLPARA + 16,
            SSLPARA + 8,
        );
    }
}

// ---------------------------------------------------------------------------
// write: EncryptMessage each chunk, send [header][data][trailer].
// ---------------------------------------------------------------------------
pub(crate) fn lower_tls_write(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    const REC: usize = 8;
    const SRC: usize = 16;
    const REMAIN: usize = 24;
    const STATE: usize = 32;
    const SENDBUF: usize = 40; // arena scratch: header+data+trailer
    const CHUNK: usize = 48;
    const FD: usize = 56;
    const BUFS: usize = 64; // SecBuffer[4] (64)
    const DESC: usize = 128;
    const ORIGLEN: usize = 144; // original data length (the write result)
    const FRAME_SIZE: usize = 0x100;

    let closed = format!("{symbol}_closed");
    let fail = format!("{symbol}_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let wloop = format!("{symbol}_wloop");
    let wdone = format!("{symbol}_wdone");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    // return_register = resource; ARG[1] = data (String/List).
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::store_u64(&v9, abi::stack_pointer(), STATE),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD),
    ]);
    // bug-508: honour the payload form. The old comment here claimed "String/List
    // OF Byte both carry [u64 len][bytes]" and read every payload as a String;
    // a `List OF Byte` block's word at +0 is its header, so its length came out
    // as ~16 MiB and the data pointer landed inside the header (OOB read, remote
    // crash of every Windows HTTPS server that wrote a byte-list body). The view
    // is now the one every other backend uses (bug-497), header check included.
    let bad_payload = format!("{symbol}_bad_payload");
    push_write_payload_view(
        &mut ins,
        text,
        abi::c_arg(1),
        &v10,
        &v11,
        &v12,
        &v13,
        &v14,
        REMAIN,
        SRC,
        &bad_payload,
    );
    // result = original length (`v10` is the length in both forms).
    ins.push(abi::store_u64(&v10, abi::stack_pointer(), ORIGLEN));
    // Allocate a send buffer sized header + maxmsg + trailer.
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE),
        abi::load_u32(&v11, &v10, st::HEADER),
        abi::load_u32(&v12, &v10, st::MAXMSG),
        abi::load_u32(&v13, &v10, st::TRAILER),
        abi::add_registers(abi::return_register(), &v11, &v12),
        abi::add_registers(abi::return_register(), abi::return_register(), &v13),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), SENDBUF));

    ins.extend([
        abi::label(&wloop),
        abi::load_u64(&v10, abi::stack_pointer(), REMAIN),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&wdone),
        // chunk = min(remaining, maxmsg)
        abi::load_u64(&v11, abi::stack_pointer(), STATE),
        abi::load_u32(&v12, &v11, st::MAXMSG),
        abi::compare_registers(&v10, &v12),
        abi::branch_le(&format!("{symbol}_ck")),
        abi::move_register(&v10, &v12),
        abi::label(&format!("{symbol}_ck")),
        abi::store_u64(&v10, abi::stack_pointer(), CHUNK),
        // copy plaintext to SENDBUF + header
        abi::load_u64(&v13, abi::stack_pointer(), STATE),
        abi::load_u32(&v14, &v13, st::HEADER),
        abi::load_u64(&v6, abi::stack_pointer(), SENDBUF),
        abi::add_registers(&v6, &v6, &v14), // dst = sendbuf+header
        abi::load_u64(&v7, abi::stack_pointer(), SRC),
    ]);
    move_bytes(&v7, &v6, &v10, &format!("{symbol}_cpin"), &mut ins, &mut vregs);
    // SecBuffers: [0]=HEADER{header, sendbuf}, [1]=DATA{chunk, sendbuf+header},
    //   [2]=TRAILER{trailer, sendbuf+header+chunk}, [3]=EMPTY.
    ins.extend([
        abi::load_u64(&v13, abi::stack_pointer(), STATE),
        abi::load_u32(&v14, &v13, st::HEADER),
        abi::load_u64(&v6, abi::stack_pointer(), SENDBUF),
        abi::store_u32(&v14, abi::stack_pointer(), BUFS),
        abi::move_immediate(&v9, "Integer", SECBUFFER_STREAM_HEADER),
        abi::store_u32(&v9, abi::stack_pointer(), BUFS + 4),
        abi::store_u64(&v6, abi::stack_pointer(), BUFS + 8),
        // [1] DATA
        abi::load_u64(&v10, abi::stack_pointer(), CHUNK),
        abi::store_u32(&v10, abi::stack_pointer(), BUFS + 16),
        abi::move_immediate(&v9, "Integer", SECBUFFER_DATA),
        abi::store_u32(&v9, abi::stack_pointer(), BUFS + 20),
        abi::add_registers(&v7, &v6, &v14),
        abi::store_u64(&v7, abi::stack_pointer(), BUFS + 24),
        // [2] TRAILER
        abi::load_u32(&v11, &v13, st::TRAILER),
        abi::store_u32(&v11, abi::stack_pointer(), BUFS + 32),
        abi::move_immediate(&v9, "Integer", SECBUFFER_STREAM_TRAILER),
        abi::store_u32(&v9, abi::stack_pointer(), BUFS + 36),
        abi::add_registers(&v7, &v7, &v10),
        abi::store_u64(&v7, abi::stack_pointer(), BUFS + 40),
    ]);
    set_secbuffer(abi::stack_pointer(), BUFS + 48, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins, &mut vregs);
    set_secbuffer_desc(abi::stack_pointer(), DESC, "4", BUFS, &mut ins, &mut vregs);
    // EncryptMessage(&ctxt, 0, &desc, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), abi::return_register(), st::CTXT),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), DESC),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    sspi_call(symbol, "EncryptMessage", SECUR32, 4, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));
    // send header+data+trailer = sum of the three cbBuffer.
    ins.extend([
        abi::load_u32(&v6, abi::stack_pointer(), BUFS),
        abi::load_u32(&v7, abi::stack_pointer(), BUFS + 16),
        abi::add_registers(&v6, &v6, &v7),
        abi::load_u32(&v7, abi::stack_pointer(), BUFS + 32),
        abi::add_registers(&v6, &v6, &v7), // total len
        abi::load_u64(&v7, abi::stack_pointer(), SENDBUF), // buf
    ]);
    send_all(symbol, FD, &v7, &v6, "enc", &fail, imports, platform, &mut ins, &mut rel, &mut vregs)?;
    // advance src/remaining
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), CHUNK),
        abi::load_u64(&v11, abi::stack_pointer(), SRC),
        abi::add_registers(&v11, &v11, &v10),
        abi::store_u64(&v11, abi::stack_pointer(), SRC),
        abi::load_u64(&v11, abi::stack_pointer(), REMAIN),
        abi::subtract_registers(&v11, &v11, &v10),
        abi::store_u64(&v11, abi::stack_pointer(), REMAIN),
        abi::branch(&wloop),
        abi::label(&wdone),
        // result = original length (saved before the loop clobbered ARG[1]).
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), ORIGLEN),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    if !text {
        // bug-497/bug-508: a byte-form payload whose header is not a `List OF Byte`'s.
        ins.push(abi::label(&bad_payload));
        emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    }
    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME_SIZE))
}

#[cfg(test)]
mod write_payload_tests {
    use super::*;
    use crate::codegen::engine::tests::TestPlatform;
    use std::collections::HashMap;

    /// Every load off the payload register (`c_arg(1)`), as `(mnemonic, offset)`.
    fn payload_loads(text: bool) -> Vec<(String, String)> {
        let (ins, _rel, _frame) = lower_tls_write("t_w", &HashMap::new(), &TestPlatform, text)
            .expect("lower schannel tls write");
        let payload = abi::c_arg(1).render();
        ins.iter()
            .filter(|i| i.get("base").as_deref() == Some(payload.as_str()))
            .map(|i| {
                (
                    i.op.mnemonic().to_string(),
                    i.get("offset").as_deref().unwrap_or("").to_string(),
                )
            })
            .collect()
    }

    // bug-508: the byte form read the payload with the String layout (`length @
    // +0`, `data @ +8`). A `List OF Byte` block's word at +0 is its header, so the
    // write length was ~16 MiB and the data pointer landed inside the header —
    // an out-of-bounds read a remote peer triggers on every Windows HTTPS server
    // that writes a byte-list body. The length must come from the collection
    // COUNT, and the header must be checked first (bug-497).
    #[test]
    fn byte_list_payload_length_is_the_collection_count_not_the_first_word() {
        let loads = payload_loads(false);
        let u64_offsets: Vec<&str> = loads
            .iter()
            .filter(|(op, _)| op == "ldr_u64")
            .map(|(_, off)| off.as_str())
            .collect();
        assert!(
            !u64_offsets.contains(&"0"),
            "bug-508: the byte form reads the word at +0 of a List OF Byte — that is the \
             String layout, and on a collection block it is the header: {loads:?}"
        );
        assert!(
            u64_offsets.contains(&COLLECTION_OFFSET_COUNT.to_string().as_str()),
            "bug-508: the byte form never reads the collection count: {loads:?}"
        );
        let mut u8_offsets: Vec<&str> = loads
            .iter()
            .filter(|(op, _)| op == "ldr_u8")
            .map(|(_, off)| off.as_str())
            .collect();
        u8_offsets.sort_unstable();
        assert_eq!(
            u8_offsets,
            vec!["0", "1", "2"],
            "bug-497: the byte form must verify kind/keyType/valueType before trusting the \
             count: {loads:?}"
        );
    }

    // The String form keeps its layout — `length @ +0` — and never reads header
    // bytes (there are none; a String's first bytes are its length).
    #[test]
    fn string_payload_length_is_the_first_word() {
        let loads = payload_loads(true);
        assert!(
            loads.iter().any(|(op, off)| op == "ldr_u64" && off == "0"),
            "the text form must read the String length at +0: {loads:?}"
        );
        assert!(
            !loads.iter().any(|(op, _)| op == "ldr_u8"),
            "the text form must not read collection header bytes: {loads:?}"
        );
    }
}

include!("gen_schannel_read_close.rs");
