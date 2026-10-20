//! plan-176-S: `crypto::sign`'s windows lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_sign/mod.rs`.

use super::*;

// ===========================================================================
// Windows CNG clean-room emit helpers (reproduced; not calls into native).
// ===========================================================================

/// Encode one big-endian `field`-wide integer at `[src]` as an ASN.1 INTEGER into
/// `[dst]`, advancing `dst` past the written bytes.
fn win_der_encode_int(
    sc: &Sc,
    src: impl Into<Reg>,
    dst: impl Into<Reg>,
    field: usize,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let (src, dst): (Reg, Reg) = (src.into(), dst.into());
    let skip = format!("{tag}_lz");
    let skip_done = format!("{tag}_lzd");
    let no_pad = format!("{tag}_np");
    ins.extend([
        abi::move_register(&sc.v9, src),
        abi::move_immediate(&sc.v10, "Integer", "0"),
        abi::move_immediate(&sc.v11, "Integer", &(field - 1).to_string()),
        abi::label(&skip),
        abi::compare_registers(&sc.v10, &sc.v11),
        abi::branch_eq(&skip_done),
        abi::load_u8(&sc.v12, &sc.v9, 0),
        abi::compare_immediate(&sc.v12, "0"),
        abi::branch_ne(&skip_done),
        abi::add_immediate(&sc.v9, &sc.v9, 1),
        abi::add_immediate(&sc.v10, &sc.v10, 1),
        abi::branch(&skip),
        abi::label(&skip_done),
        abi::move_immediate(&sc.v13, "Integer", &field.to_string()),
        abi::subtract_registers(&sc.v13, &sc.v13, &sc.v10),
        abi::load_u8(&sc.v12, &sc.v9, 0),
        abi::shift_right_immediate(&sc.v14, &sc.v12, 7),
        abi::add_registers(&sc.v10, &sc.v13, &sc.v14),
        abi::move_immediate(&sc.v12, "Byte", "2"),
        abi::store_u8(&sc.v12, dst, 0),
        abi::store_u8(&sc.v10, dst, 1),
        abi::add_immediate(dst, dst, 2),
        abi::compare_immediate(&sc.v14, "0"),
        abi::branch_eq(&no_pad),
        abi::store_u8(abi::ZERO, dst, 0),
        abi::add_immediate(dst, dst, 1),
        abi::label(&no_pad),
    ]);
    gen_cert::win_copy_bytes(sc, &sc.v9, dst, &sc.v13, tag, ins);
}

/// Open the ECDSA provider at `halg_off` and import the SEC1 private key at
/// `[key_ptr_off]` into `hkey_off`.
#[allow(clippy::too_many_arguments)]
fn win_import_private_key(
    sc: &Sc,
    curve: SignCurve,
    symbol: &str,
    tag: &str,
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
    let magic = curve.priv_magic();
    let body_len = 3 * field;
    let blob_len = 8 + 3 * field;
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
    // Build the blob: [magic(4)][cbKey=field(4)][X‖Y‖d from SEC1+1].
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
    // BCryptImportKeyPair(hAlg, NULL, blobId, &hKey, blob, blobLen, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::win_wide_addr(symbol, abi::c_arg(2), "ECCPRIVATEBLOB", ins, rel);
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
    ins.push(abi::branch_lt(fail));
    Ok(())
}

/// Hash the message at `[msgbuf_off]`/`[msglen_off]` into `hashbuf_off` using the
/// curve's digest. Opens (and closes) its own hash provider at `hashalg_off`.
#[allow(clippy::too_many_arguments)]
fn win_hash_message(
    curve: SignCurve,
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
    let no_key = format!("{symbol}_clean_nokey_{tag}");
    let no_alg = format!("{symbol}_clean_noalg_{tag}");
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

/// Emit the Windows CNG ECDSA sign sequence for `curve`, self-contained. Imports the
/// SEC1 private key as an ECCPRIVATEBLOB, hashes the message, `BCryptSignHash`s the
/// fixed `r‖s`, and DER-encodes it. Self-managed fallible ABI; jumps `done`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_windows_sign(
    curve: SignCurve,
    symbol: &str,
    tag: &str,
    priv_op: Operand,
    msg_op: Operand,
    done: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
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
    const RS: usize = 104;
    const DERBUF: usize = 112;
    const DERSTART: usize = 120;
    const COLL: usize = 128;
    const HASHINLINE: usize = 136; // 64-byte inline hash scratch
    let blobcap = gen_cert::BLOBCAP;

    let fail = format!("{symbol}_{tag}_fail");
    let invalid = format!("{symbol}_{tag}_invalid");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");
    let cleanup = format!("{symbol}_{tag}_cleanup");

    let mut vregs = Vregs::new();
    let sc = Sc::new(&mut vregs);
    ins.extend([
        abi::store_u64(priv_op, abi::stack_pointer(), PRIVCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BLOB),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PRIVBUF),
    ]);
    emit_read_byte_list(
        symbol,
        &format!("{tag}priv"),
        PRIVCOLL,
        PRIVBUF,
        PRIVLEN,
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
    // priv len must be the SEC1 private (point ‖ scalar).
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), PRIVLEN),
        abi::compare_immediate(&sc.v9, &priv_raw.to_string()),
        abi::branch_ne(&invalid),
    ]);
    // Allocate the CNG blob + rs + der buffers.
    for (cap, slot) in [(blobcap, BLOB), (2 * 66, RS), (16 + 4 * 66, DERBUF)] {
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

    win_import_private_key(
        &sc, curve, symbol, tag, PRIVBUF, BLOB, HALG, HKEY, &fail, imports, platform, ins, rel,
    )?;
    win_hash_message(
        curve, symbol, tag, MSGBUF, MSGLEN, HASHINLINE, HASHALG, &fail, imports, platform, ins, rel,
    )?;

    // BCryptSignHash(hKey, NULL, hash, hashLen, rs, 2*field, &cbResult, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HASHINLINE),
        abi::move_immediate(abi::c_arg(3), "Integer", &curve.hash_len().to_string()),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), RS),
        abi::move_immediate(abi::c_arg(5), "Integer", &(2 * field).to_string()),
        abi::add_immediate(abi::c_arg(6), abi::stack_pointer(), CBRES),
        abi::move_immediate(abi::c_arg(7), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(symbol, "BCryptSignHash", 8, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(&fail));

    // DER-encode: body at DERBUF+3; r at rs+0, s at rs+field.
    ins.extend([
        abi::load_u64(&sc.v15, abi::stack_pointer(), DERBUF),
        abi::add_immediate(&sc.v15, &sc.v15, 3), // body cursor (dst)
        abi::load_u64(&sc.v7, abi::stack_pointer(), RS), // r src
    ]);
    win_der_encode_int(
        &sc,
        &sc.v7,
        &sc.v15,
        field,
        &format!("{symbol}_{tag}r"),
        ins,
    );
    ins.extend([
        abi::load_u64(&sc.v7, abi::stack_pointer(), RS),
        abi::add_immediate(&sc.v7, &sc.v7, field), // s src
    ]);
    win_der_encode_int(
        &sc,
        &sc.v7,
        &sc.v15,
        field,
        &format!("{symbol}_{tag}s"),
        ins,
    );
    // total body len = %v15 - (DERBUF+3)
    let short = format!("{symbol}_{tag}_short");
    let hdr_done = format!("{symbol}_{tag}_hdrdone");
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), DERBUF),
        abi::add_immediate(&sc.v10, &sc.v9, 3),
        abi::subtract_registers(&sc.v11, &sc.v15, &sc.v10), // total body len
        abi::compare_immediate(&sc.v11, "128"),
        abi::branch_lt(&short),
        // long form: [0x30][0x81][len] at DERBUF+0; start=DERBUF, len=total+3
        abi::move_immediate(&sc.v12, "Byte", "48"),
        abi::store_u8(&sc.v12, &sc.v9, 0),
        abi::move_immediate(&sc.v12, "Integer", "129"),
        abi::store_u8(&sc.v12, &sc.v9, 1),
        abi::store_u8(&sc.v11, &sc.v9, 2),
        abi::store_u64(&sc.v9, abi::stack_pointer(), DERSTART),
        abi::add_immediate(&sc.v11, &sc.v11, 3),
        abi::store_u64(&sc.v11, abi::stack_pointer(), DERLEN),
        abi::branch(&hdr_done),
        abi::label(&short),
        // short form: [0x30][len] at DERBUF+1; start=DERBUF+1, len=total+2
        abi::add_immediate(&sc.v13, &sc.v9, 1),
        abi::move_immediate(&sc.v12, "Byte", "48"),
        abi::store_u8(&sc.v12, &sc.v13, 0),
        abi::store_u8(&sc.v11, &sc.v13, 1),
        abi::store_u64(&sc.v13, abi::stack_pointer(), DERSTART),
        abi::add_immediate(&sc.v11, &sc.v11, 2),
        abi::store_u64(&sc.v11, abi::stack_pointer(), DERLEN),
        abi::label(&hdr_done),
    ]);

    // Destroy the CNG handles BEFORE building the result (the cleanup calls clobber
    // the caller-saved result registers). win_cleanup nulls the handle slots.
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
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_{tag}_out_loop"),
        &format!("{symbol}_{tag}_out_done"),
        DERSTART,
        DERLEN,
        Some(COLL),
        abi::mfb_return(1),
        &alloc_fail,
        &mut vregs,
        ins,
        rel,
    );
    ins.push(abi::branch(&cleanup)); // wipe_and_done

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
    emit_fail(symbol, "ErrUnknown", ins, rel, &cleanup);
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
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, &cleanup);
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
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, &cleanup);

    // wipe_and_done: the CNG handles are already destroyed on every incoming path;
    // the private-buffer wipes are call-free, so they don't disturb the result regs.
    ins.push(abi::label(&cleanup));
    emit_zero_guarded(
        symbol,
        PRIVBUF,
        Some(PRIVLEN),
        priv_raw,
        &format!("{tag}privz"),
        &mut vregs,
        ins,
    );
    emit_zero_guarded(
        symbol,
        BLOB,
        None,
        blobcap,
        &format!("{tag}blobz"),
        &mut vregs,
        ins,
    );
    ins.push(abi::branch(done));
    Ok(())
}
