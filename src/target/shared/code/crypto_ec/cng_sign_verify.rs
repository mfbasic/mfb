// Included into cng.rs. Sign/verify plus the ASN.1 DER encode/decode of the
// ECDSA (r, s) pair. Kept in a separate file only to bound cng.rs's length.

/// Encode one big-endian `field`-wide integer at `[src]` as an ASN.1 INTEGER into
/// `[dst]`, advancing `dst` past the written bytes. `src`/`dst` are register
/// operands consumed in place. Scratch: `%v8`..`%v14`.
fn der_encode_int(src: &str, dst: &str, field: usize, tag: &str, ins: &mut Vec<CodeInstruction>) {
    let skip = format!("{tag}_lz");
    let skip_done = format!("{tag}_lzd");
    let no_pad = format!("{tag}_np");
    // Find the first significant byte: keep at least one byte even if all-zero.
    ins.extend([
        abi::move_register("%v9", src),            // working ptr into src
        abi::move_immediate("%v10", "Integer", "0"), // i
        abi::move_immediate("%v11", "Integer", &(field - 1).to_string()),
        abi::label(&skip),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_eq(&skip_done),
        abi::load_u8("%v12", "%v9", 0),
        abi::compare_immediate("%v12", "0"),
        abi::branch_ne(&skip_done),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&skip),
        abi::label(&skip_done),
        // sig_len = field - i  → %v13
        abi::move_immediate("%v13", "Integer", &field.to_string()),
        abi::subtract_registers("%v13", "%v13", "%v10"),
        // need_pad = (src[i] & 0x80) != 0 → %v14 ∈ {0,1}
        abi::load_u8("%v12", "%v9", 0),
        abi::shift_right_immediate("%v14", "%v12", 7),
        // content_len = sig_len + need_pad → %v10 (reuse)
        abi::add_registers("%v10", "%v13", "%v14"),
        // dst[0]=0x02, dst[1]=content_len
        abi::move_immediate("%v12", "Byte", "2"),
        abi::store_u8("%v12", dst, 0),
        abi::store_u8("%v10", dst, 1),
        abi::add_immediate(dst, dst, 2),
        // if need_pad: dst[0]=0x00; dst++
        abi::compare_immediate("%v14", "0"),
        abi::branch_eq(&no_pad),
        abi::store_u8(abi::ZERO, dst, 0),
        abi::add_immediate(dst, dst, 1),
        abi::label(&no_pad),
    ]);
    // copy sig_len (%v13) bytes from %v9 (src+i) to dst.
    copy_bytes("%v9", dst, "%v13", tag, ins);
}

/// Decode one ASN.1 INTEGER at `[body]` into the big-endian `field`-wide slot at
/// `[dst]` (left-padded with zeros). Advances `body` past the INTEGER. Branches to
/// `fail` on a malformed tag / oversized value. Scratch: `%v8`..`%v14`.
fn der_decode_int(
    body: &str,
    dst: &str,
    field: usize,
    tag: &str,
    fail: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let no_pad = format!("{tag}_dnp");
    let ok = format!("{tag}_dok");
    ins.extend([
        // tag byte must be 0x02
        abi::load_u8("%v9", body, 0),
        abi::compare_immediate("%v9", "2"),
        abi::branch_ne(fail),
        abi::load_u8("%v10", body, 1), // declared length
        abi::add_immediate("%v11", body, 2), // int body ptr
        // advance `body` past this INTEGER now (2 + declared_len), before trimming.
        abi::add_immediate(body, body, 2),
        abi::add_registers(body, body, "%v10"),
        // if int_body[0]==0 and len>1: skip the pad byte
        abi::load_u8("%v9", "%v11", 0),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&no_pad),
        abi::compare_immediate("%v10", "1"),
        abi::branch_eq(&no_pad),
        abi::add_immediate("%v11", "%v11", 1),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::label(&no_pad),
        // len must be <= field
        abi::move_immediate("%v12", "Integer", &field.to_string()),
        abi::compare_registers("%v10", "%v12"),
        abi::branch_hi(fail),
    ]);
    // zero the dst field, then copy len bytes to dst + (field - len).
    ins.extend([
        abi::move_immediate("%v13", "Integer", "0"),
        abi::move_register("%v14", dst),
        abi::label(&format!("{tag}_zl")),
        abi::compare_registers("%v13", "%v12"),
        abi::branch_eq(&format!("{tag}_zld")),
        abi::store_u8(abi::ZERO, "%v14", 0),
        abi::add_immediate("%v14", "%v14", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&format!("{tag}_zl")),
        abi::label(&format!("{tag}_zld")),
        // dst_off = dst + (field - len)
        abi::subtract_registers("%v12", "%v12", "%v10"),
        abi::move_register("%v14", dst),
        abi::add_registers("%v14", "%v14", "%v12"),
    ]);
    copy_bytes("%v11", "%v14", "%v10", tag, ins);
    ins.push(abi::label(&ok));
    let _ = ok;
}

/// Open the ECDSA algorithm provider at `halg_off` and import the SEC1 key at
/// `[key_ptr_off]` (public or private) into `hkey_off`. `is_private` selects the
/// blob type/magic and copies the scalar. Branches to `fail` on any NTSTATUS < 0.
#[allow(clippy::too_many_arguments)]
fn import_key(
    curve: Curve,
    is_private: bool,
    symbol: &str,
    key_ptr_off: usize,
    blob_off: usize,
    halg_off: usize,
    hkey_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let field = curve.field_len();
    let (magic, blob_id, body_len, blob_len) = if is_private {
        (curve.priv_magic(), "ECCPRIVATEBLOB", 3 * field, 8 + 3 * field)
    } else {
        (curve.pub_magic(), "ECCPUBLICBLOB", 2 * field, 8 + 2 * field)
    };
    // Open ECDSA provider.
    ins.push(abi::add_immediate(abi::return_register(), abi::stack_pointer(), halg_off));
    wide_addr(symbol, abi::ARG[1], curve.algo_id(), ins, rel);
    ins.extend([
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptOpenAlgorithmProvider", 4, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(fail));
    // Build the blob: [magic(4)][cbKey=field(4)][X‖Y(‖d) from SEC1+1].
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), blob_off),
        abi::move_immediate("%v9", "Integer", magic),
        abi::store_u32("%v9", "%v10", 0),
        abi::move_immediate("%v9", "Integer", &field.to_string()),
        abi::store_u32("%v9", "%v10", 4),
        abi::load_u64("%v11", abi::stack_pointer(), key_ptr_off),
        abi::add_immediate("%v11", "%v11", 1), // skip the 0x04 SEC1 prefix
        abi::add_immediate("%v12", "%v10", 8),
        abi::move_immediate("%v13", "Integer", &body_len.to_string()),
    ]);
    copy_bytes("%v11", "%v12", "%v13", &format!("{symbol}_ik"), ins);
    // BCryptImportKeyPair(hAlg, NULL, blobId, &hKey, blob, blobLen, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    wide_addr(symbol, abi::ARG[2], blob_id, ins, rel);
    ins.extend([
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), hkey_off),
        abi::load_u64(abi::ARG[4], abi::stack_pointer(), blob_off),
        abi::move_immediate(abi::ARG[5], "Integer", &blob_len.to_string()),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptImportKeyPair", 7, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(fail));
    Ok(())
}

/// Hash the message at `[msgbuf_off]`/`[msglen_off]` into `hashbuf_off` using the
/// curve's digest. Opens (and closes) its own hash provider at `hashalg_off`.
#[allow(clippy::too_many_arguments)]
fn hash_message(
    curve: Curve,
    symbol: &str,
    msgbuf_off: usize,
    msglen_off: usize,
    hashbuf_off: usize,
    hashalg_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    ins.push(abi::add_immediate(abi::return_register(), abi::stack_pointer(), hashalg_off));
    wide_addr(symbol, abi::ARG[1], curve.hash_id(), ins, rel);
    ins.extend([
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptOpenAlgorithmProvider", 4, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(fail));
    // BCryptHash(hAlg, NULL, 0, msg, msgLen, hash, hashLen)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hashalg_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::load_u64(abi::ARG[3], abi::stack_pointer(), msgbuf_off),
        abi::load_u64(abi::ARG[4], abi::stack_pointer(), msglen_off),
        abi::add_immediate(abi::ARG[5], abi::stack_pointer(), hashbuf_off),
        abi::move_immediate(abi::ARG[6], "Integer", &curve.hash_len().to_string()),
    ]);
    bcrypt_call(symbol, "BCryptHash", 7, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(fail));
    // Close the hash provider.
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hashalg_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptCloseAlgorithmProvider", 2, imports, platform, ins, rel)?;
    Ok(())
}

fn sign(
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let field = curve.field_len();
    let priv_raw = 1 + 3 * field;
    const PRIVCOLL: usize = 0;
    const MSGCOLL: usize = 8;
    const PRIVBUF: usize = 16;
    const PRIVLEN: usize = 24;
    const MSGBUF: usize = 32;
    const MSGLEN: usize = 40;
    const HALG: usize = 48;
    const HKEY: usize = 56;
    const HASHALG: usize = 64;
    const BLOB: usize = 72;
    const CBRES: usize = 88;
    const DERLEN: usize = 96;
    const RS: usize = 104; // ptr
    const DERBUF: usize = 112; // ptr
    const DERSTART: usize = 120; // ptr
    const COLL: usize = 128;
    const HASHINLINE: usize = 136; // 64-byte inline hash scratch (must fit LOCAL_SIZE)
    const BLOBCAP: usize = 8 + 3 * 66;
    const LOCAL_SIZE: usize = 136 + 64;

    let fail = format!("{symbol}_fail");
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let cleanup = format!("{symbol}_cleanup");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PRIVCOLL),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MSGCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BLOB),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PRIVBUF),
    ]);
    emit_read_byte_list(symbol, "priv", PRIVCOLL, PRIVBUF, PRIVLEN, &alloc_fail, &mut ins, &mut rel);
    emit_read_byte_list(symbol, "msg", MSGCOLL, MSGBUF, MSGLEN, &alloc_fail, &mut ins, &mut rel);
    // priv len must be the SEC1 private (point ‖ scalar).
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), PRIVLEN),
        abi::compare_immediate("%v9", &priv_raw.to_string()),
        abi::branch_ne(&invalid),
    ]);
    // Allocate the CNG blob + rs + der buffers.
    for (cap, slot) in [(BLOBCAP, BLOB), (2 * 66, RS), (16 + 4 * 66, DERBUF)] {
        ins.extend([
            abi::move_immediate(abi::return_register(), "Integer", &cap.to_string()),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), slot));
    }

    import_key(curve, true, symbol, PRIVBUF, BLOB, HALG, HKEY, &fail, imports, platform, &mut ins, &mut rel)?;
    hash_message(curve, symbol, MSGBUF, MSGLEN, HASHINLINE, HASHALG, &fail, imports, platform, &mut ins, &mut rel)?;

    // BCryptSignHash(hKey, NULL, hash, hashLen, rs, 2*field, &cbResult, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HASHINLINE),
        abi::move_immediate(abi::ARG[3], "Integer", &curve.hash_len().to_string()),
        abi::load_u64(abi::ARG[4], abi::stack_pointer(), RS),
        abi::move_immediate(abi::ARG[5], "Integer", &(2 * field).to_string()),
        abi::add_immediate(abi::ARG[6], abi::stack_pointer(), CBRES),
        abi::move_immediate(abi::ARG[7], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptSignHash", 8, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));

    // DER-encode: body at DERBUF+3; r at rs+0, s at rs+field.
    ins.extend([
        abi::load_u64("%v15", abi::stack_pointer(), DERBUF),
        abi::add_immediate("%v15", "%v15", 3), // body cursor (dst)
        abi::load_u64("%v7", abi::stack_pointer(), RS), // r src
    ]);
    // encode r (src=%v7, dst=%v15). copy_bytes/der use %v8-%v14; keep %v7/%v15 live.
    der_encode_int("%v7", "%v15", field, &format!("{symbol}_r"), &mut ins);
    ins.extend([
        abi::load_u64("%v7", abi::stack_pointer(), RS),
        abi::add_immediate("%v7", "%v7", field), // s src
    ]);
    der_encode_int("%v7", "%v15", field, &format!("{symbol}_s"), &mut ins);
    // total body len = %v15 - (DERBUF+3)
    let short = format!("{symbol}_short");
    let hdr_done = format!("{symbol}_hdrdone");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), DERBUF),
        abi::add_immediate("%v10", "%v9", 3),
        abi::subtract_registers("%v11", "%v15", "%v10"), // total body len
        abi::compare_immediate("%v11", "128"),
        abi::branch_lt(&short),
        // long form: [0x30][0x81][len] at DERBUF+0; start=DERBUF, len=total+3
        abi::move_immediate("%v12", "Byte", "48"),
        abi::store_u8("%v12", "%v9", 0),
        abi::move_immediate("%v12", "Integer", "129"),
        abi::store_u8("%v12", "%v9", 1),
        abi::store_u8("%v11", "%v9", 2),
        abi::store_u64("%v9", abi::stack_pointer(), DERSTART),
        abi::add_immediate("%v11", "%v11", 3),
        abi::store_u64("%v11", abi::stack_pointer(), DERLEN),
        abi::branch(&hdr_done),
        abi::label(&short),
        // short form: [0x30][len] at DERBUF+1; start=DERBUF+1, len=total+2
        abi::add_immediate("%v13", "%v9", 1),
        abi::move_immediate("%v12", "Byte", "48"),
        abi::store_u8("%v12", "%v13", 0),
        abi::store_u8("%v11", "%v13", 1),
        abi::store_u64("%v13", abi::stack_pointer(), DERSTART),
        abi::add_immediate("%v11", "%v11", 2),
        abi::store_u64("%v11", abi::stack_pointer(), DERLEN),
        abi::label(&hdr_done),
    ]);

    // Destroy the CNG handles (clobbers caller-saved result regs) BEFORE building
    // the result. emit_cleanup nulls the handle slots so the error paths reuse it.
    emit_cleanup(symbol, "cok", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_out_loop"),
        &format!("{symbol}_out_done"),
        DERSTART,
        DERLEN,
        Some(COLL),
        abi::RET[1],
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::branch(&cleanup)); // cleanup=wipe_and_done; BCrypt cleanup done above

    ins.push(abi::label(&fail));
    emit_cleanup(symbol, "cf", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_UNKNOWN_CODE, ERR_UNKNOWN_SYMBOL, &mut ins, &mut rel, &cleanup);
    ins.push(abi::label(&invalid));
    emit_cleanup(symbol, "ci", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut ins, &mut rel, &cleanup);
    ins.push(abi::label(&alloc_fail));
    emit_cleanup(symbol, "ca", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &cleanup);

    // wipe_and_done: the CNG handles are already destroyed on every incoming path;
    // the private-buffer wipes are call-free, so they don't disturb the result regs.
    ins.push(abi::label(&cleanup));
    emit_zero_guarded(symbol, PRIVBUF, Some(PRIVLEN), priv_raw, "privz", &mut ins);
    emit_zero_guarded(symbol, BLOB, None, BLOBCAP, "blobz", &mut ins);
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}

fn verify(
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let field = curve.field_len();
    let pub_raw = 1 + 2 * field;
    const PUBCOLL: usize = 0;
    const MSGCOLL: usize = 8;
    const SIGCOLL: usize = 16;
    const PUBBUF: usize = 24;
    const PUBLEN: usize = 32;
    const MSGBUF: usize = 40;
    const MSGLEN: usize = 48;
    const SIGBUF: usize = 56;
    const SIGLEN: usize = 64;
    const HALG: usize = 72;
    const HKEY: usize = 80;
    const HASHALG: usize = 88;
    const BLOB: usize = 96;
    const RS: usize = 104; // ptr
    const HASHINLINE: usize = 112; // 64 bytes
    const LOCAL_SIZE: usize = 112 + 64;
    const BLOBCAP: usize = 8 + 2 * 66;

    let fail = format!("{symbol}_fail");
    let bad_sig = format!("{symbol}_badsig");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let cleanup = format!("{symbol}_cleanup");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PUBCOLL),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MSGCOLL),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), SIGCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HKEY),
    ]);
    emit_read_byte_list(symbol, "pub", PUBCOLL, PUBBUF, PUBLEN, &alloc_fail, &mut ins, &mut rel);
    emit_read_byte_list(symbol, "msg", MSGCOLL, MSGBUF, MSGLEN, &alloc_fail, &mut ins, &mut rel);
    emit_read_byte_list(symbol, "sig", SIGCOLL, SIGBUF, SIGLEN, &alloc_fail, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), PUBLEN),
        abi::compare_immediate("%v9", &pub_raw.to_string()),
        abi::branch_ne(&bad_sig),
    ]);
    for (cap, slot) in [(BLOBCAP, BLOB), (2 * 66, RS)] {
        ins.extend([
            abi::move_immediate(abi::return_register(), "Integer", &cap.to_string()),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), slot));
    }

    import_key(curve, false, symbol, PUBBUF, BLOB, HALG, HKEY, &fail, imports, platform, &mut ins, &mut rel)?;
    hash_message(curve, symbol, MSGBUF, MSGLEN, HASHINLINE, HASHALG, &fail, imports, platform, &mut ins, &mut rel)?;

    // DER-decode the signature into rs (r at +0, s at +field), zero-padded.
    let seq_short = format!("{symbol}_seqshort");
    let seq_body = format!("{symbol}_seqbody");
    ins.extend([
        abi::load_u64("%v15", abi::stack_pointer(), SIGBUF),
        abi::load_u8("%v9", "%v15", 0),
        abi::compare_immediate("%v9", "48"), // SEQUENCE
        abi::branch_ne(&bad_sig),
        abi::load_u8("%v9", "%v15", 1),
        abi::compare_immediate("%v9", "128"),
        abi::branch_lt(&seq_short),
        // long form 0x81: body starts at +3
        abi::compare_immediate("%v9", "129"),
        abi::branch_ne(&bad_sig),
        abi::add_immediate("%v15", "%v15", 3),
        abi::branch(&seq_body),
        abi::label(&seq_short),
        abi::add_immediate("%v15", "%v15", 2),
        abi::label(&seq_body),
        abi::load_u64("%v6", abi::stack_pointer(), RS), // dst for r
    ]);
    der_decode_int("%v15", "%v6", field, &format!("{symbol}_dr"), &bad_sig, &mut ins);
    ins.extend([
        abi::load_u64("%v6", abi::stack_pointer(), RS),
        abi::add_immediate("%v6", "%v6", field), // dst for s
    ]);
    der_decode_int("%v15", "%v6", field, &format!("{symbol}_ds"), &bad_sig, &mut ins);

    // BCryptVerifySignature(hKey, NULL, hash, hashLen, rs, 2*field, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HASHINLINE),
        abi::move_immediate(abi::ARG[3], "Integer", &curve.hash_len().to_string()),
        abi::load_u64(abi::ARG[4], abi::stack_pointer(), RS),
        abi::move_immediate(abi::ARG[5], "Integer", &(2 * field).to_string()),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptVerifySignature", 7, imports, platform, &mut ins, &mut rel)?;
    // Destroy the CNG handles (clobbers result regs) BEFORE recording the verdict.
    // The NTSTATUS is preserved in a callee-safe vreg across the cleanup calls.
    ins.push(abi::move_register("%v7", abi::return_register()));
    emit_cleanup(symbol, "cok", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    // status == 0 → valid; anything else (incl STATUS_INVALID_SIGNATURE) → false.
    ins.extend([
        abi::compare_immediate("%v7", "0"),
        abi::branch_ne(&bad_sig),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // bad_sig is reached both after the (already-cleaned-up) verify and from the
    // earlier len/DER-decode guards where the handles may still be open — the
    // idempotent cleanup covers both.
    ins.push(abi::label(&bad_sig));
    emit_cleanup(symbol, "cb", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&fail));
    emit_cleanup(symbol, "cf", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_UNKNOWN_CODE, ERR_UNKNOWN_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_cleanup(symbol, "ca", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);

    let _ = cleanup;
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}
