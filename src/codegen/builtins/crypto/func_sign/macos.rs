//! plan-176-S: `crypto::sign`'s macos lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_sign/mod.rs`.

use super::*;

/// Emit the macOS SecKey ECDSA sign sequence for `curve`, self-contained. The two
/// argument collection pointers arrive in `priv_op`/`msg_op`. Success leaves the DER
/// signature `List OF Byte` in the result registers and branches `done`; failure
/// paths release the CF objects and `emit_fail` to `done`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_macos_sign(
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
    const SEC: usize = 0;
    const CF: usize = 8;
    const FN: usize = 16;
    const RELEASE: usize = 24;
    const PRIVCOLL: usize = 32;
    const MSGCOLL: usize = 40;
    const PRIVBUF: usize = 48;
    const PRIVLEN: usize = 56;
    const MSGBUF: usize = 64;
    const MSGLEN: usize = 72;
    const PRIVDATA: usize = 80;
    const MSGDATA: usize = 88;
    const KEY: usize = 96;
    const SIGDATA: usize = 104;
    const DICT: usize = 112;
    const ALGO: usize = 120;
    const BYTEPTR: usize = 128;
    const BYTELEN: usize = 136;
    const COLL: usize = 144;
    const SCRATCH: usize = 152; // 6 slots: 152..200
    const CONST_SCRATCH: usize = 200;

    let load_fail = format!("{symbol}_{tag}_load_fail");
    let invalid_fail = format!("{symbol}_{tag}_invalid_fail");
    let sign_fail = format!("{symbol}_{tag}_sign_fail");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");

    let mut vregs = Vregs::new();
    let v9 = vregs.next();

    // Stash the two collection arguments before anything clobbers the arg registers.
    ins.extend([
        abi::store_u64(priv_op, abi::stack_pointer(), PRIVCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
    ]);
    // Zero the CF object slots and the private-scalar scratch pointer so the
    // error-exit cleanup can null-guard each CFRelease / wipe.
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PRIVBUF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PRIVDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MSGDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DICT),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), KEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SIGDATA),
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

    // privData = CFDataCreate(NULL, privBuf, privLen); msgData = CFDataCreate(...)
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
    gen_cert::cfdata_create(FN, PRIVBUF, PRIVLEN, PRIVDATA, &v9, ins);
    gen_cert::cfdata_create(FN, MSGBUF, MSGLEN, MSGDATA, &v9, ins);

    gen_cert::build_dict2(
        symbol,
        SEC,
        CF,
        FN,
        "kSecAttrKeyType",
        "kSecAttrKeyClass",
        "kSecAttrKeyTypeECSECPrimeRandom",
        "kSecAttrKeyClassPrivate",
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

    // key = SecKeyCreateWithData(privData, dict, NULL)
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
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PRIVDATA),
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

    // sigData = SecKeyCreateSignature(key, algo, msgData, NULL)
    gen_cert::dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateSignature",
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
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SIGDATA),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&sign_fail),
    ]);

    gen_cert::cfdata_to_list(
        symbol,
        tag,
        CF,
        SIGDATA,
        FN,
        BYTEPTR,
        BYTELEN,
        COLL,
        &load_fail,
        &alloc_fail,
        imports,
        platform,
        &v9,
        &mut vregs,
        ins,
        rel,
    )?;

    gen_cert::cf_release(RELEASE, PRIVDATA, &v9, ins);
    gen_cert::cf_release(RELEASE, MSGDATA, &v9, ins);
    gen_cert::cf_release(RELEASE, DICT, &v9, ins);
    gen_cert::cf_release(RELEASE, KEY, &v9, ins);
    gen_cert::cf_release(RELEASE, SIGDATA, &v9, ins);
    emit_zero_guarded(
        symbol,
        PRIVBUF,
        Some(PRIVLEN),
        0,
        &format!("{tag}privS"),
        &mut vregs,
        ins,
    );

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    let cleanup = |ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs, ctag: &str| {
        gen_cert::cf_release_guarded(symbol, RELEASE, PRIVDATA, &format!("{ctag}p"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, MSGDATA, &format!("{ctag}m"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, DICT, &format!("{ctag}d"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, KEY, &format!("{ctag}k"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, SIGDATA, &format!("{ctag}s"), &v9, ins);
        emit_zero_guarded(
            symbol,
            PRIVBUF,
            Some(PRIVLEN),
            0,
            &format!("{ctag}z"),
            vregs,
            ins,
        );
    };
    ins.push(abi::label(&load_fail));
    cleanup(ins, &mut vregs, &format!("{tag}lf"));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&sign_fail));
    cleanup(ins, &mut vregs, &format!("{tag}sf"));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&invalid_fail));
    cleanup(ins, &mut vregs, &format!("{tag}iv"));
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
    ins.push(abi::label(&alloc_fail));
    cleanup(ins, &mut vregs, &format!("{tag}af"));
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    Ok(())
}
