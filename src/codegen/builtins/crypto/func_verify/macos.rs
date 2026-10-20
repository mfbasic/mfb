//! plan-176-S: `crypto::verify`'s macos lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_verify/mod.rs`.

use super::*;

// ===========================================================================
// macOS SecKey clean-room verify (reproduced; not a call into native).
// ===========================================================================

/// Emit the macOS SecKey ECDSA verify sequence for `curve`, self-contained. The three
/// argument collection pointers arrive in `pub_op`/`msg_op`/`sig_op`. Success leaves the
/// Boolean verdict in the result registers and branches `done`; a malformed key raises
/// `ErrInvalidArgument`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_macos_verify(
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
    const SEC: usize = 0;
    const CF: usize = 8;
    const FN: usize = 16;
    const RELEASE: usize = 24;
    const PUBCOLL: usize = 32;
    const MSGCOLL: usize = 40;
    const SIGCOLL: usize = 48;
    const PUBBUF: usize = 56;
    const PUBLEN: usize = 64;
    const MSGBUF: usize = 72;
    const MSGLEN: usize = 80;
    const SIGBUF: usize = 88;
    const SIGLEN: usize = 96;
    const PUBDATA: usize = 104;
    const MSGDATA: usize = 112;
    const SIGDATA: usize = 120;
    const KEY: usize = 128;
    const DICT: usize = 136;
    const ALGO: usize = 144;
    const BOOLRES: usize = 152;
    const SCRATCH: usize = 160; // 6 slots 160..208
    const CONST_SCRATCH: usize = 208;

    let load_fail = format!("{symbol}_{tag}_load_fail");
    let invalid_fail = format!("{symbol}_{tag}_invalid_fail");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");

    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();

    // Stash the three collection arguments before anything clobbers the arg registers.
    ins.extend([
        abi::store_u64(pub_op, abi::stack_pointer(), PUBCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
        abi::store_u64(sig_op, abi::stack_pointer(), SIGCOLL),
    ]);
    // Zero the CF object slots so the error-exit cleanup can null-guard each CFRelease.
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PUBDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MSGDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SIGDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), KEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DICT),
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
    // Reject a public key that is not exactly one uncompressed SEC1 point (parity with
    // the OpenSSL/CNG backends, which check the length explicitly).
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), PUBLEN),
        abi::compare_immediate(&v9, &curve.point_len().to_string()),
        abi::branch_ne(&invalid_fail),
    ]);
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

    gen_cert::dlopen_one(
        symbol,
        gen_cert::SECPATH_SYMBOL,
        SEC,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    gen_cert::dlopen_one(
        symbol,
        gen_cert::CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFRelease",
        RELEASE,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;

    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFDataCreate",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    gen_cert::cfdata_create(FN, PUBBUF, PUBLEN, PUBDATA, &v9, ins);
    gen_cert::cfdata_create(FN, MSGBUF, MSGLEN, MSGDATA, &v9, ins);
    gen_cert::cfdata_create(FN, SIGBUF, SIGLEN, SIGDATA, &v9, ins);

    gen_cert::build_dict2(
        symbol,
        SEC,
        CF,
        FN,
        "kSecAttrKeyType",
        "kSecAttrKeyClass",
        "kSecAttrKeyTypeECSECPrimeRandom",
        "kSecAttrKeyClassPublic",
        SCRATCH,
        CONST_SCRATCH,
        DICT,
        &load_fail,
        imports,
        platform,
        &v9,
        ins,
        rel,
    )?;

    // key = SecKeyCreateWithData(pubData, dict, NULL)
    gen_cert::dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateWithData",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PUBDATA),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DICT),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    gen_cert::load_cf_const(
        symbol,
        SEC,
        curve.macos_algorithm(),
        ALGO,
        CONST_SCRATCH,
        &load_fail,
        imports,
        platform,
        &v9,
        ins,
        rel,
    )?;

    // ok = SecKeyVerifySignature(key, algo, msgData, sigData, NULL)
    gen_cert::dlsym_into(
        symbol,
        SEC,
        "SecKeyVerifySignature",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), ALGO),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MSGDATA),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), SIGDATA),
        abi::move_immediate(abi::c_arg(4), "Integer", "0"),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    // Normalise the CF `Boolean` (a 0/1 byte with unspecified upper bits) to a clean
    // 0/1 by masking bit 0. A false verify is a legitimate Boolean, not an error.
    ins.extend([
        abi::move_immediate(&v10, "Integer", "1"),
        abi::and_registers(&v9, abi::return_register(), &v10),
        abi::store_u64(&v9, abi::stack_pointer(), BOOLRES),
    ]);

    gen_cert::cf_release(RELEASE, PUBDATA, &v9, ins);
    gen_cert::cf_release(RELEASE, MSGDATA, &v9, ins);
    gen_cert::cf_release(RELEASE, SIGDATA, &v9, ins);
    gen_cert::cf_release(RELEASE, DICT, &v9, ins);
    gen_cert::cf_release(RELEASE, KEY, &v9, ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), BOOLRES),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    let cleanup = |ins: &mut Vec<CodeInstruction>, ctag: &str| {
        gen_cert::cf_release_guarded(symbol, RELEASE, PUBDATA, &format!("{ctag}p"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, MSGDATA, &format!("{ctag}m"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, SIGDATA, &format!("{ctag}s"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, DICT, &format!("{ctag}d"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, KEY, &format!("{ctag}k"), &v9, ins);
    };
    ins.push(abi::label(&load_fail));
    cleanup(ins, &format!("{tag}lf"));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&invalid_fail));
    cleanup(ins, &format!("{tag}iv"));
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
    ins.push(abi::label(&alloc_fail));
    cleanup(ins, &format!("{tag}af"));
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    Ok(())
}
