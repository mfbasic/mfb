//! plan-176-S: `crypto::sign`'s linux lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_sign/mod.rs`.

use super::*;

// ===========================================================================
// Linux OpenSSL clean-room emit helpers (reproduced; not calls into native).
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

/// Emit the Linux OpenSSL ECDSA sign sequence for `curve`, self-contained. The
/// private key is spliced into a fixed PKCS#8 template and driven through
/// `d2i_AutoPrivateKey` + one-shot `EVP_DigestSign`. Self-managed fallible ABI.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_linux_sign(
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
    const HANDLE: usize = 0;
    const FN: usize = 8;
    const PRIVCOLL: usize = 16;
    const MSGCOLL: usize = 24;
    const PRIVBUF: usize = 32;
    const PRIVLEN: usize = 40;
    const MSGBUF: usize = 48;
    const MSGLEN: usize = 56;
    const DERBUF: usize = 64;
    const DERPP: usize = 72;
    const TMPLPTR: usize = 80;
    const PKEY: usize = 88;
    const MDCTX: usize = 96;
    const MD: usize = 104;
    const SIGLEN: usize = 112;
    const SIGBUF: usize = 120;
    const COLL: usize = 128;

    let pkcs8_len = curve.pkcs8_len();
    let point_len = curve.point_len();
    let field_len = curve.field_len();
    let p8_point_off = curve.p8_point_off();
    let p8_scalar_off = curve.p8_scalar_off();

    let load_fail = format!("{symbol}_{tag}_load_fail");
    let invalid_fail = format!("{symbol}_{tag}_invalid_fail");
    let sign_fail = format!("{symbol}_{tag}_sign_fail");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");
    let raw_fail = format!("{symbol}_{tag}_raw_fail");

    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();

    ins.extend([
        abi::store_u64(priv_op, abi::stack_pointer(), PRIVCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
    ]);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MDCTX),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PRIVBUF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DERBUF),
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
        abi::load_u64(&v9, abi::stack_pointer(), PRIVLEN),
        abi::compare_immediate(&v9, &(point_len + field_len).to_string()),
        abi::branch_ne(&invalid_fail),
    ]);

    gen_cert::dlopen_libcrypto(symbol, tag, HANDLE, &load_fail, imports, platform, ins, rel)?;

    // privDer = template with point/scalar spliced from the raw key bytes.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &pkcs8_len.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), DERBUF),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), DERPP),
    ]);
    emit_data_address(symbol, &v9, &gen_cert::tmpl_sym(curve.name()), ins, rel);
    ins.push(abi::store_u64(&v9, abi::stack_pointer(), TMPLPTR));
    gen_cert::copy_bytes(
        symbol,
        &format!("{tag}tmpl"),
        TMPLPTR,
        0,
        None,
        DERBUF,
        0,
        None,
        CopyLen::Const(pkcs8_len),
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );
    // raw key = 0x04||X||Y||K = point(point_len) || scalar(field_len)
    gen_cert::copy_bytes(
        symbol,
        &format!("{tag}pt"),
        PRIVBUF,
        0,
        None,
        DERBUF,
        p8_point_off,
        None,
        CopyLen::Const(point_len),
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );
    gen_cert::copy_bytes(
        symbol,
        &format!("{tag}sc"),
        PRIVBUF,
        point_len,
        None,
        DERBUF,
        p8_scalar_off,
        None,
        CopyLen::Const(field_len),
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );

    // pkey = d2i_AutoPrivateKey(NULL, &pp, len)
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "d2i_AutoPrivateKey",
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
        abi::move_immediate(abi::c_arg(2), "Integer", &pkcs8_len.to_string()),
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

    // EVP_MD_CTX_new returns NULL on malloc failure; route to the generic exit.
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);

    // EVP_DigestSignInit(ctx, NULL, md, NULL, pkey)
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestSignInit",
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
    ins.extend([
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&sign_fail),
    ]);

    // siglen probe: EVP_DigestSign(ctx, NULL, &siglen, msg, msglen)
    gen_cert::ossl_dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestSign",
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
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), SIGLEN),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), MSGBUF),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), MSGLEN),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    ins.extend([
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&sign_fail),
    ]);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SIGLEN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, &alloc_fail);
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        SIGBUF,
    ));
    // EVP_DigestSign(ctx, sig, &siglen, msg, msglen)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SIGBUF),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), SIGLEN),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), MSGBUF),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), MSGLEN),
    ]);
    gen_cert::call_fn(FN, &v9, ins);
    ins.extend([
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&sign_fail),
    ]);

    emit_build_byte_list(
        symbol,
        &format!("{symbol}_{tag}_out_build_loop"),
        &format!("{symbol}_{tag}_out_build_done"),
        SIGBUF,
        SIGLEN,
        Some(COLL),
        abi::mfb_return(1),
        &alloc_fail,
        &mut vregs,
        ins,
        rel,
    );

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
    // Wipe the raw private scalar and the spliced PKCS#8 DER (both hold the key).
    emit_zero_guarded(
        symbol,
        PRIVBUF,
        Some(PRIVLEN),
        0,
        &format!("{tag}privS"),
        &mut vregs,
        ins,
    );
    emit_zero_guarded(
        symbol,
        DERBUF,
        None,
        pkcs8_len,
        &format!("{tag}derS"),
        &mut vregs,
        ins,
    );

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
    let cleanup = |ins: &mut Vec<CodeInstruction>,
                   rel: &mut Vec<CodeRelocation>,
                   vregs: &mut Vregs,
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
        emit_zero_guarded(
            symbol,
            PRIVBUF,
            Some(PRIVLEN),
            0,
            &format!("{ctag}pz"),
            vregs,
            ins,
        );
        emit_zero_guarded(
            symbol,
            DERBUF,
            None,
            pkcs8_len,
            &format!("{ctag}dz"),
            vregs,
            ins,
        );
        Ok(())
    };
    ins.push(abi::label(&load_fail));
    cleanup(ins, rel, &mut vregs, &format!("{tag}lf"))?;
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&sign_fail));
    cleanup(ins, rel, &mut vregs, &format!("{tag}sf"))?;
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&invalid_fail));
    cleanup(ins, rel, &mut vregs, &format!("{tag}iv"))?;
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
    ins.push(abi::label(&alloc_fail));
    cleanup(ins, rel, &mut vregs, &format!("{tag}af"))?;
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    // raw_fail: a free's own dlsym failed (unreachable once libcrypto is loaded).
    ins.push(abi::label(&raw_fail));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    Ok(())
}
