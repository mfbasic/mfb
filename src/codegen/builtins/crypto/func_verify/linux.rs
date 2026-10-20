//! plan-176-S: `crypto::verify`'s linux lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_verify/mod.rs`.

use super::*;

// ===========================================================================
// Linux OpenSSL clean-room verify (reproduced; not a call into native).
// ===========================================================================

/// Free the object at `obj_off` via `free_name` only when the slot is non-NULL.
#[allow(clippy::too_many_arguments)]
fn lx_free_guarded(
    symbol: &str,
    handle_off: usize,
    obj_off: usize,
    free_name: &str,
    fn_off: usize,
    tag: &str,
    raw_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    scratch: impl Into<Reg>,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let scratch: Reg = scratch.into();
    let skip = format!("{symbol}_{tag}_nofree");
    ins.extend([
        abi::load_u64(scratch, abi::stack_pointer(), obj_off),
        abi::compare_immediate(scratch, "0"),
        abi::branch_eq(&skip),
    ]);
    gen_cert::ossl_dlsym_into(
        symbol, handle_off, free_name, fn_off, raw_fail, imports, platform, ins, rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        obj_off,
    ));
    gen_cert::call_fn(fn_off, scratch, ins);
    ins.push(abi::label(&skip));
    Ok(())
}

/// Emit the Linux OpenSSL ECDSA verify sequence for `curve`, self-contained. The public
/// key is prefixed with a fixed SPKI DER header, decoded via `d2i_PUBKEY`, and checked
/// with one-shot `EVP_DigestVerify`. Self-managed fallible ABI.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_linux_verify(
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
    const HANDLE: usize = 0;
    const FN: usize = 8;
    const PUBCOLL: usize = 16;
    const MSGCOLL: usize = 24;
    const SIGCOLL: usize = 32;
    const PUBBUF: usize = 40;
    const PUBLEN: usize = 48;
    const MSGBUF: usize = 56;
    const MSGLEN: usize = 64;
    const SIGBUF: usize = 72;
    const SIGLEN: usize = 80;
    const DERBUF: usize = 88;
    const DERPP: usize = 96;
    const PREFPTR: usize = 104;
    const PKEY: usize = 112;
    const MDCTX: usize = 120;
    const MD: usize = 128;
    const BOOLRES: usize = 136;

    let point_len = curve.point_len();
    let prefix_len = gen_cert::spki_prefix_len(curve.name());
    let der_len = prefix_len + point_len;

    let load_fail = format!("{symbol}_{tag}_load_fail");
    let invalid_fail = format!("{symbol}_{tag}_invalid_fail");
    let verify_fail = format!("{symbol}_{tag}_verify_fail");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");
    let raw_fail = format!("{symbol}_{tag}_raw_fail");
    let vtrue = format!("{symbol}_{tag}_vtrue");
    let vstore = format!("{symbol}_{tag}_vstore");

    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();

    ins.extend([
        abi::store_u64(pub_op, abi::stack_pointer(), PUBCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
        abi::store_u64(sig_op, abi::stack_pointer(), SIGCOLL),
    ]);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MDCTX),
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
    // pub len must be exactly one uncompressed SEC1 point.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), PUBLEN),
        abi::compare_immediate(&v9, &point_len.to_string()),
        abi::branch_ne(&invalid_fail),
    ]);

    gen_cert::dlopen_libcrypto(symbol, tag, HANDLE, &load_fail, imports, platform, ins, rel)?;

    // pubDer = spki_prefix || point
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &der_len.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), DERBUF),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), DERPP),
    ]);
    emit_data_address(symbol, &v9, &gen_cert::spki_sym(curve.name()), ins, rel);
    ins.push(abi::store_u64(&v9, abi::stack_pointer(), PREFPTR));
    gen_cert::copy_bytes(
        symbol,
        &format!("{tag}pref"),
        PREFPTR,
        0,
        None,
        DERBUF,
        0,
        None,
        CopyLen::Const(prefix_len),
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );
    gen_cert::copy_bytes(
        symbol,
        &format!("{tag}pt"),
        PUBBUF,
        0,
        None,
        DERBUF,
        prefix_len,
        None,
        CopyLen::Const(point_len),
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );

    // pkey = d2i_PUBKEY(NULL, &pp, der_len)
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "d2i_PUBKEY",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), DERPP),
        abi::move_immediate(abi::c_arg(2), "Integer", &der_len.to_string()),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    ins.extend([
        // An external C call returns in the C-return bank (`rax`), not the aligned
        // MFB-return bank (`rdi`) these bodies otherwise read; on x86-64 SysV the two
        // differ, so the result must be read from `c_return(0)` (byte-identical on
        // AArch64/RISC-V, where both banks are `x0`). See bug-450.
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), PKEY),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_MD_CTX_new",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    gen_cert::call_fn(FN, &v9, ins);
    // C result in `c_return(0)` (bug-450).
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        MDCTX,
    ));
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        curve.digest(),
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    gen_cert::call_fn(FN, &v9, ins);
    // C result in `c_return(0)` (bug-450).
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), MD));
    // EVP_MD_CTX_new returns NULL on OOM; route to the generic exit before it is
    // dereferenced by EVP_DigestVerifyInit.
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);

    // EVP_DigestVerifyInit(ctx, NULL, md, NULL, pkey)
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestVerifyInit",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MD),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), PKEY),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    // An init failure is a real error, not a silent "invalid signature" verdict.
    // C result in `c_return(0)` (bug-450).
    ins.extend([
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&verify_fail),
    ]);

    // rc = EVP_DigestVerify(ctx, sig, siglen, msg, msglen); valid iff rc == 1.
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestVerify",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SIGBUF),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), SIGLEN),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), MSGBUF),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), MSGLEN),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    // C result (the verify verdict) in `c_return(0)` (bug-450).
    ins.extend([
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_eq(&vtrue),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::branch(&vstore),
        abi::label(&vtrue),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::label(&vstore),
        abi::store_u64(&v9, abi::stack_pointer(), BOOLRES),
    ]);

    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_MD_CTX_free",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MDCTX,
    ));
    gen_cert::call_fn(FN, &v9, ins);
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_PKEY_free",
        FN,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        PKEY,
    ));
    gen_cert::call_fn(FN, &v9, ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), BOOLRES),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
    let cleanup = |ins: &mut Vec<CodeInstruction>,
                   rel: &mut Vec<CodeRelocation>,
                   ctag: &str|
     -> Result<(), String> {
        lx_free_guarded(
            symbol,
            HANDLE,
            MDCTX,
            "EVP_MD_CTX_free",
            FN,
            &format!("{ctag}mc"),
            &raw_fail,
            imports,
            platform,
            &v9,
            ins,
            rel,
        )?;
        lx_free_guarded(
            symbol,
            HANDLE,
            PKEY,
            "EVP_PKEY_free",
            FN,
            &format!("{ctag}pk"),
            &raw_fail,
            imports,
            platform,
            &v9,
            ins,
            rel,
        )?;
        Ok(())
    };
    ins.push(abi::label(&load_fail));
    cleanup(ins, rel, &format!("{tag}lf"))?;
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&verify_fail));
    cleanup(ins, rel, &format!("{tag}vf"))?;
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&invalid_fail));
    cleanup(ins, rel, &format!("{tag}iv"))?;
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
    ins.push(abi::label(&alloc_fail));
    cleanup(ins, rel, &format!("{tag}af"))?;
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    // raw_fail: a free's own dlsym failed (unreachable once libcrypto is loaded).
    ins.push(abi::label(&raw_fail));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    Ok(())
}
