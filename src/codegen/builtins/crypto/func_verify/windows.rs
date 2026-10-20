//! plan-176-S: `crypto::verify`'s windows lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_verify/mod.rs`.

use super::*;

// ===========================================================================
// Windows CNG clean-room verify (reproduced; not a call into native).
// ===========================================================================

/// Decode one ASN.1 INTEGER at `[body]` into the big-endian `field`-wide slot at `[dst]`
/// (left-padded with zeros). Advances `body` past the INTEGER. Branches to `fail` on a
/// malformed tag / oversized value, and to `bounds_fail` on any length that would read
/// past the untrusted signature buffer (`[sigbuf_off]`..+`[siglen_off]`). Scratch:
/// `sc.v8`..`sc.v14`.
#[allow(clippy::too_many_arguments)]
fn win_der_decode_int(
    sc: &Sc,
    body: impl Into<Reg>,
    dst: impl Into<Reg>,
    field: usize,
    tag: &str,
    fail: &str,
    sigbuf_off: usize,
    siglen_off: usize,
    bounds_fail: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let (body, dst): (Reg, Reg) = (body.into(), dst.into());
    let no_pad = format!("{tag}_dnp");
    ins.extend([
        // Bound the 2-byte tag+length header against the signature buffer end.
        abi::load_u64(&sc.v8, abi::stack_pointer(), sigbuf_off),
        abi::load_u64(&sc.v9, abi::stack_pointer(), siglen_off),
        abi::add_registers(&sc.v8, &sc.v8, &sc.v9), // %v8 = buffer end
        abi::add_immediate(&sc.v9, body, 2),
        abi::compare_registers(&sc.v9, &sc.v8),
        abi::branch_hi(bounds_fail),
        // tag byte must be 0x02
        abi::load_u8(&sc.v9, body, 0),
        abi::compare_immediate(&sc.v9, "2"),
        abi::branch_ne(fail),
        abi::load_u8(&sc.v10, body, 1),       // declared length
        abi::add_immediate(&sc.v11, body, 2), // int body ptr
        // advance `body` past this INTEGER now (2 + declared_len), before trimming.
        abi::add_immediate(body, body, 2),
        abi::add_registers(body, body, &sc.v10),
        // Reject an empty INTEGER and bound the declared content.
        abi::compare_immediate(&sc.v10, "0"),
        abi::branch_eq(bounds_fail),
        abi::load_u64(&sc.v8, abi::stack_pointer(), sigbuf_off),
        abi::load_u64(&sc.v9, abi::stack_pointer(), siglen_off),
        abi::add_registers(&sc.v8, &sc.v8, &sc.v9), // %v8 = buffer end
        abi::compare_registers(body, &sc.v8),
        abi::branch_hi(bounds_fail),
        // if int_body[0]==0 and len>1: skip the pad byte
        abi::load_u8(&sc.v9, &sc.v11, 0),
        abi::compare_immediate(&sc.v9, "0"),
        abi::branch_ne(&no_pad),
        abi::compare_immediate(&sc.v10, "1"),
        abi::branch_eq(&no_pad),
        abi::add_immediate(&sc.v11, &sc.v11, 1),
        abi::subtract_immediate(&sc.v10, &sc.v10, 1),
        abi::label(&no_pad),
        // len must be <= field
        abi::move_immediate(&sc.v12, "Integer", &field.to_string()),
        abi::compare_registers(&sc.v10, &sc.v12),
        abi::branch_hi(fail),
    ]);
    // zero the dst field, then copy len bytes to dst + (field - len).
    ins.extend([
        abi::move_immediate(&sc.v13, "Integer", "0"),
        abi::move_register(&sc.v14, dst),
        abi::label(&format!("{tag}_zl")),
        abi::compare_registers(&sc.v13, &sc.v12),
        abi::branch_eq(&format!("{tag}_zld")),
        abi::store_u8(abi::ZERO, &sc.v14, 0),
        abi::add_immediate(&sc.v14, &sc.v14, 1),
        abi::add_immediate(&sc.v13, &sc.v13, 1),
        abi::branch(&format!("{tag}_zl")),
        abi::label(&format!("{tag}_zld")),
        // dst_off = dst + (field - len)
        abi::subtract_registers(&sc.v12, &sc.v12, &sc.v10),
        abi::move_register(&sc.v14, dst),
        abi::add_registers(&sc.v14, &sc.v14, &sc.v12),
    ]);
    gen_cert::win_copy_bytes(sc, &sc.v11, &sc.v14, &sc.v10, tag, ins);
}

/// Open the ECDSA provider at `halg_off` and import the SEC1 public key at
/// `[key_ptr_off]` into `hkey_off` as an `ECCPUBLICBLOB`. Branches to `fail` if the
/// provider cannot be opened (a system error), and to `import_fail` if the blob fails to
/// import (an off-curve / otherwise-invalid key → `ErrInvalidArgument`).
#[allow(clippy::too_many_arguments)]
fn win_import_public_key(
    sc: &Sc,
    curve: VerifyCurve,
    symbol: &str,
    tag: &str,
    key_ptr_off: usize,
    blob_off: usize,
    halg_off: usize,
    hkey_off: usize,
    fail: &str,
    import_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let field = curve.field_len();
    let magic = curve.pub_magic();
    let body_len = 2 * field;
    let blob_len = 8 + 2 * field;
    // Open ECDSA provider.
    ins.push(abi::add_immediate(
        abi::return_register(),
        abi::stack_pointer(),
        halg_off,
    ));
    gen_cert::win_wide_addr(symbol, abi::c_arg(1), curve.algo_id(), ins, rel);
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptOpenAlgorithmProvider",
        4,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::branch_lt(fail));
    // Build the blob: [magic(4)][cbKey=field(4)][X‖Y from SEC1+1].
    ins.extend([
        abi::load_u64(&sc.v10, abi::stack_pointer(), blob_off),
        abi::move_immediate(&sc.v9, "Integer", magic),
        abi::store_u32(&sc.v9, &sc.v10, 0),
        abi::move_immediate(&sc.v9, "Integer", &field.to_string()),
        abi::store_u32(&sc.v9, &sc.v10, 4),
        abi::load_u64(&sc.v11, abi::stack_pointer(), key_ptr_off),
        abi::add_immediate(&sc.v11, &sc.v11, 1), // skip the 0x04 SEC1 prefix
        abi::add_immediate(&sc.v12, &sc.v10, 8),
        abi::move_immediate(&sc.v13, "Integer", &body_len.to_string()),
    ]);
    gen_cert::win_copy_bytes(
        sc,
        &sc.v11,
        &sc.v12,
        &sc.v13,
        &format!("{symbol}_{tag}_ik"),
        ins,
    );
    // BCryptImportKeyPair(hAlg, NULL, ECCPUBLICBLOB, &hKey, blob, blobLen, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::win_wide_addr(symbol, abi::c_arg(2), "ECCPUBLICBLOB", ins, rel);
    ins.extend([
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), hkey_off),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), blob_off),
        abi::move_immediate(abi::c_arg(5), "Integer", &blob_len.to_string()),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptImportKeyPair",
        7,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::branch_lt(import_fail));
    Ok(())
}

/// Hash the message at `[msgbuf_off]`/`[msglen_off]` into `hashbuf_off` using the curve's
/// digest. Opens (and closes) its own hash provider at `hashalg_off`.
#[allow(clippy::too_many_arguments)]
fn win_hash_message(
    curve: VerifyCurve,
    symbol: &str,
    tag: &str,
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
    ins.push(abi::add_immediate(
        abi::return_register(),
        abi::stack_pointer(),
        hashalg_off,
    ));
    gen_cert::win_wide_addr(symbol, abi::c_arg(1), curve.hash_id(), ins, rel);
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptOpenAlgorithmProvider",
        4,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::branch_lt(fail));
    // BCryptHash(hAlg, NULL, 0, msg, msgLen, hash, hashLen)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hashalg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), msgbuf_off),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), msglen_off),
        abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), hashbuf_off),
        abi::move_immediate(abi::c_arg(6), "Integer", &curve.hash_len().to_string()),
    ]);
    let hash_fail = format!("{symbol}_{tag}_hashfail");
    let hash_ok = format!("{symbol}_{tag}_hashok");
    gen_cert::bcrypt_call(symbol, "BCryptHash", 7, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(&hash_fail));
    // Close the hash provider (success path).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hashalg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptCloseAlgorithmProvider",
        2,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::branch(&hash_ok));
    // Failure path: close the hash provider before routing to the caller's fail exit.
    ins.push(abi::label(&hash_fail));
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hashalg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptCloseAlgorithmProvider",
        2,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::branch(fail));
    ins.push(abi::label(&hash_ok));
    Ok(())
}

/// Destroy `hKey` (at `hkey_off`) and close `hAlg` (at `halg_off`), each null-guarded.
#[allow(clippy::too_many_arguments)]
fn win_cleanup(
    symbol: &str,
    tag: &str,
    hkey_off: usize,
    halg_off: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let no_key = format!("{symbol}_vclean_nokey_{tag}");
    let no_alg = format!("{symbol}_vclean_noalg_{tag}");
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hkey_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_key),
    ]);
    gen_cert::bcrypt_call(symbol, "BCryptDestroyKey", 1, imports, platform, ins, rel)?;
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), hkey_off));
    ins.push(abi::label(&no_key));
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_alg),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptCloseAlgorithmProvider",
        2,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), halg_off));
    ins.push(abi::label(&no_alg));
    Ok(())
}

/// Emit the Windows CNG ECDSA verify sequence for `curve`, self-contained. Imports the
/// SEC1 public key as an ECCPUBLICBLOB, hashes the message, DER-decodes the peer
/// signature into a fixed `r‖s`, and calls `BCryptVerifySignature`. Self-managed
/// fallible ABI; jumps `done`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_windows_verify(
    curve: VerifyCurve,
    symbol: &str,
    tag: &str,
    pub_op: Operand,
    msg_op: Operand,
    sig_op: Operand,
    done: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
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
    const RS: usize = 104;
    const HASHINLINE: usize = 112; // 64-byte inline hash scratch (112..176)
    let blobcap = 8 + 2 * 66;

    let fail = format!("{symbol}_{tag}_fail");
    let bad_sig = format!("{symbol}_{tag}_badsig");
    let invalid = format!("{symbol}_{tag}_invalid");
    let oob = format!("{symbol}_{tag}_oob");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");

    let mut vregs = Vregs::new();
    let sc = Sc::new(&mut vregs);
    ins.extend([
        abi::store_u64(pub_op, abi::stack_pointer(), PUBCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
        abi::store_u64(sig_op, abi::stack_pointer(), SIGCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HKEY),
    ]);
    emit_read_byte_list(
        symbol,
        &format!("{tag}pub"),
        PUBCOLL,
        PUBBUF,
        PUBLEN,
        &alloc_fail,
        &mut vregs,
        ins,
        rel,
    );
    emit_read_byte_list(
        symbol,
        &format!("{tag}msg"),
        MSGCOLL,
        MSGBUF,
        MSGLEN,
        &alloc_fail,
        &mut vregs,
        ins,
        rel,
    );
    emit_read_byte_list(
        symbol,
        &format!("{tag}sig"),
        SIGCOLL,
        SIGBUF,
        SIGLEN,
        &alloc_fail,
        &mut vregs,
        ins,
        rel,
    );
    // A public key that is not exactly one uncompressed SEC1 point is a malformed
    // argument, not a false verdict.
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), PUBLEN),
        abi::compare_immediate(&sc.v9, &pub_raw.to_string()),
        abi::branch_ne(&invalid),
    ]);
    for (cap, slot) in [(blobcap, BLOB), (2 * 66, RS)] {
        ins.extend([
            abi::move_immediate(abi::return_register(), "Integer", &cap.to_string()),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        emit_alloc(symbol, ins, rel, &alloc_fail);
        ins.push(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            slot,
        ));
    }

    win_import_public_key(
        &sc, curve, symbol, tag, PUBBUF, BLOB, HALG, HKEY, &fail, &invalid, imports, platform, ins,
        rel,
    )?;
    win_hash_message(
        curve, symbol, tag, MSGBUF, MSGLEN, HASHINLINE, HASHALG, &fail, imports, platform, ins, rel,
    )?;

    // DER-decode the untrusted signature into rs (r at +0, s at +field), zero-padded,
    // bounding every read against SIGLEN. Reading the SEQUENCE header needs >= 2 bytes.
    let seq_short = format!("{symbol}_{tag}_seqshort");
    let seq_body = format!("{symbol}_{tag}_seqbody");
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), SIGLEN),
        abi::compare_immediate(&sc.v9, "2"),
        abi::branch_lt(&oob),
        abi::load_u64(&sc.v15, abi::stack_pointer(), SIGBUF),
        abi::load_u8(&sc.v9, &sc.v15, 0),
        abi::compare_immediate(&sc.v9, "48"), // SEQUENCE
        abi::branch_ne(&bad_sig),
        abi::load_u8(&sc.v9, &sc.v15, 1),
        abi::compare_immediate(&sc.v9, "128"),
        abi::branch_lt(&seq_short),
        // long form 0x81: body starts at +3
        abi::compare_immediate(&sc.v9, "129"),
        abi::branch_ne(&bad_sig),
        abi::add_immediate(&sc.v15, &sc.v15, 3),
        abi::branch(&seq_body),
        abi::label(&seq_short),
        abi::add_immediate(&sc.v15, &sc.v15, 2),
        abi::label(&seq_body),
        abi::load_u64(&sc.v6, abi::stack_pointer(), RS), // dst for r
    ]);
    win_der_decode_int(
        &sc,
        &sc.v15,
        &sc.v6,
        field,
        &format!("{symbol}_{tag}dr"),
        &bad_sig,
        SIGBUF,
        SIGLEN,
        &oob,
        ins,
    );
    ins.extend([
        abi::load_u64(&sc.v6, abi::stack_pointer(), RS),
        abi::add_immediate(&sc.v6, &sc.v6, field), // dst for s
    ]);
    win_der_decode_int(
        &sc,
        &sc.v15,
        &sc.v6,
        field,
        &format!("{symbol}_{tag}ds"),
        &bad_sig,
        SIGBUF,
        SIGLEN,
        &oob,
        ins,
    );

    // BCryptVerifySignature(hKey, NULL, hash, hashLen, rs, 2*field, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HASHINLINE),
        abi::move_immediate(abi::c_arg(3), "Integer", &curve.hash_len().to_string()),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), RS),
        abi::move_immediate(abi::c_arg(5), "Integer", &(2 * field).to_string()),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptVerifySignature",
        7,
        imports,
        platform,
        ins,
        rel,
    )?;
    // bug-446: the NTSTATUS is a C result — it lands in the C-return register
    // (`c_return(0)` = `rax`), NOT `return_register()` (the aligned Win64 MFB result
    // bank = `rcx`, which still holds the stale first argument, the key handle). Read
    // the status from `c_return(0)` and sign-extend it out of the 32-bit `LONG` into a
    // callee-safe vreg that survives the cleanup calls (which clobber `rax`). The old
    // cng verify read `return_register()` = the non-zero handle → `!= 0` → always FALSE.
    ins.push(abi::sign_extend_word(&sc.v7, abi::c_return(0)));
    win_cleanup(
        symbol,
        &format!("{tag}cok"),
        HKEY,
        HALG,
        imports,
        platform,
        ins,
        rel,
    )?;
    // status == 0 → valid; anything else (incl STATUS_INVALID_SIGNATURE) → FALSE.
    ins.extend([
        abi::compare_immediate(&sc.v7, "0"),
        abi::branch_ne(&bad_sig),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    // An out-of-bounds / short DER encoding is just an invalid signature (FALSE),
    // detected before any OOB read; fall through into bad_sig's cleanup+verdict.
    ins.push(abi::label(&oob));
    ins.push(abi::label(&bad_sig));
    win_cleanup(
        symbol,
        &format!("{tag}cb"),
        HKEY,
        HALG,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
    ins.push(abi::label(&fail));
    win_cleanup(
        symbol,
        &format!("{tag}cf"),
        HKEY,
        HALG,
        imports,
        platform,
        ins,
        rel,
    )?;
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    // A malformed public key (wrong length, or a right-length off-curve key that
    // BCryptImportKeyPair rejects) is an argument error.
    ins.push(abi::label(&invalid));
    win_cleanup(
        symbol,
        &format!("{tag}ci"),
        HKEY,
        HALG,
        imports,
        platform,
        ins,
        rel,
    )?;
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
    ins.push(abi::label(&alloc_fail));
    win_cleanup(
        symbol,
        &format!("{tag}ca"),
        HKEY,
        HALG,
        imports,
        platform,
        ins,
        rel,
    )?;
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    Ok(())
}
