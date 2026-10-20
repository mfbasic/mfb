//! plan-176-S: `crypto::generate`'s linux lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_generate/mod.rs`.

use super::*;

// Linux stack layout (sp-relative).
const L_HANDLE: usize = 0;

const L_FN: usize = 8;

const L_PKEY: usize = 16;

const L_ECKEY: usize = 24;

const L_FREEPKEY: usize = 32;

const L_FREEECKEY: usize = 40;

const L_SEC1PTR: usize = 48;

const L_SEC1LEN: usize = 56;

const L_SEC1PP: usize = 64;

const L_SPKIPTR: usize = 72;

const L_SPKILEN: usize = 80;

const L_SPKIPP: usize = 88;

const L_RAWBUF: usize = 96;

const L_RAWLEN: usize = 104;

const L_COLL: usize = 112;

const L_PUBCOLL: usize = 120;

const L_RSIZE: usize = 128;

const L_RRESULT: usize = 136;

const L_RCURSOR: usize = 144;

const L_RBLOCK: usize = 152;

pub(super) const L_NID: usize = 160;

pub(super) const L_POINTLEN: usize = 168;

pub(super) const L_FIELDLEN: usize = 176;

pub(super) const L_SEC1OFF: usize = 184;

pub(super) const L_SPKIPREFIX: usize = 192;

pub(super) const L_NAMEPTR: usize = 200;

/// Emit the Linux OpenSSL EC keygen for the runtime curve params staged in the
/// `L_NID`/`L_POINTLEN`/`L_FIELDLEN`/`L_SEC1OFF`/`L_SPKIPREFIX`/`L_NAMEPTR` slots.
/// Self-managed fallible ABI; jumps `done`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_linux_ec(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    symbol: &str,
    v9: impl Into<Reg>,
    v10: impl Into<Reg>,
    v11: impl Into<Reg>,
    v12: impl Into<Reg>,
    v13: impl Into<Reg>,
    vregs: &mut Vregs,
    done: &str,
) -> Result<(), String> {
    let (v9, v10, v11, v12, v13): (Reg, Reg, Reg, Reg, Reg) =
        (v9.into(), v10.into(), v11.into(), v12.into(), v13.into());
    let imports = ctx.platform_imports;
    let platform = ctx.platform;
    let load_fail = format!("{symbol}_load_fail");
    let gen_fail = format!("{symbol}_gen_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let eckey_path = format!("{symbol}_eckey");
    let have_pkey = format!("{symbol}_have_pkey");
    let ins = &mut builder.instructions;

    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_PKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_ECKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_SEC1PTR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_COLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_PUBCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_SPKIPTR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_RAWBUF),
    ]);

    gen_cert::dlopen_libcrypto(
        symbol,
        "g",
        L_HANDLE,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    // Resolve the free functions up front so the error cleanup can always run.
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "EVP_PKEY_free",
        L_FREEPKEY,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "EC_KEY_free",
        L_FREEECKEY,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;

    // OpenSSL 3.x: pkey = EVP_EC_gen(name).
    gen_cert::ossl_dlsym_probe(
        symbol,
        L_HANDLE,
        "EVP_EC_gen",
        L_FN,
        &eckey_path,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_NAMEPTR),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // An external C call returns in the C-return bank (`rax`), not the aligned
        // MFB-return bank (`rdi`) these bodies otherwise read; on x86-64 SysV the two
        // differ, so the result must be read from `c_return(0)` (byte-identical on
        // AArch64/RISC-V, where both banks are `x0`). See bug-450.
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), L_PKEY),
        abi::branch(&have_pkey),
    ]);

    // OpenSSL 1.1: EC_KEY_new_by_curve_name(nid) + generate + EVP_PKEY_assign.
    ins.push(abi::label(&eckey_path));
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "EC_KEY_new_by_curve_name",
        L_FN,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_NID),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result in `c_return(0)` (bug-450): `rax` on x86-64, `x0` on AArch64.
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), L_ECKEY),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&gen_fail),
    ]);
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "EC_KEY_generate_key",
        L_FN,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_ECKEY),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result in `c_return(0)` (bug-450).
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&gen_fail),
    ]);
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "EVP_PKEY_new",
        L_FN,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result in `c_return(0)` (bug-450).
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), L_PKEY),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&gen_fail),
    ]);
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "EVP_PKEY_assign",
        L_FN,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), L_PKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", gen_cert::EVP_PKEY_EC),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), L_ECKEY),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result in `c_return(0)` (bug-450).
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&gen_fail),
        // Ownership transferred to pkey; clear ECKEY so cleanup does not double-free.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), L_ECKEY),
    ]);

    ins.push(abi::label(&have_pkey));
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // SEC1 = i2d_PrivateKey(pkey)
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "i2d_PrivateKey",
        L_FN,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result (the SEC1 length) in `c_return(0)` (bug-450).
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), L_SEC1LEN),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_le(&gen_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_SEC1LEN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, &mut builder.relocations, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), L_SEC1PTR),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), L_SEC1PP),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), L_SEC1PP),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result in `c_return(0)` (bug-450).
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_le(&gen_fail),
    ]);

    // SPKI = i2d_PUBKEY(pkey)
    gen_cert::ossl_dlsym_into(
        symbol,
        L_HANDLE,
        "i2d_PUBKEY",
        L_FN,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result (the SPKI length) in `c_return(0)` (bug-450).
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), L_SPKILEN),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_le(&gen_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_SPKILEN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, &mut builder.relocations, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), L_SPKIPTR),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), L_SPKIPP),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), L_SPKIPP),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        // C result in `c_return(0)` (bug-450).
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_le(&gen_fail),
    ]);

    // raw (point_len + field_len) = point || scalar
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), L_POINTLEN),
        abi::load_u64(v12, abi::stack_pointer(), L_FIELDLEN),
        abi::add_registers(v9, v9, v12),
        abi::store_u64(v9, abi::stack_pointer(), L_RAWLEN),
        abi::move_register(abi::return_register(), v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, &mut builder.relocations, &alloc_fail);
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        L_RAWBUF,
    ));

    // point = SPKI[spki_prefix..][point_len]; guard SPKILEN >= spki_prefix + point_len.
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), L_SPKILEN),
        abi::load_u64(v10, abi::stack_pointer(), L_SPKIPREFIX),
        abi::load_u64(v11, abi::stack_pointer(), L_POINTLEN),
        abi::add_registers(v10, v10, v11),
        abi::compare_registers(v9, v10),
        abi::branch_lo(&gen_fail),
    ]);
    gen_cert::copy_bytes(
        symbol,
        "pt",
        L_SPKIPTR,
        0,
        Some(L_SPKIPREFIX),
        L_RAWBUF,
        0,
        None,
        CopyLen::Runtime(L_POINTLEN),
        v9,
        v10,
        v11,
        v12,
        v13,
        ins,
    );
    // scalar = SEC1[sec1_off..][field_len]; guard SEC1LEN >= sec1_off + field_len.
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), L_SEC1LEN),
        abi::load_u64(v10, abi::stack_pointer(), L_SEC1OFF),
        abi::load_u64(v11, abi::stack_pointer(), L_FIELDLEN),
        abi::add_registers(v10, v10, v11),
        abi::compare_registers(v9, v10),
        abi::branch_lo(&gen_fail),
    ]);
    gen_cert::copy_bytes(
        symbol,
        "sc",
        L_SEC1PTR,
        0,
        Some(L_SEC1OFF),
        L_RAWBUF,
        0,
        Some(L_POINTLEN),
        CopyLen::Runtime(L_FIELDLEN),
        v9,
        v10,
        v11,
        v12,
        v13,
        ins,
    );

    emit_build_byte_list(
        symbol,
        &format!("{symbol}_priv_loop"),
        &format!("{symbol}_priv_done"),
        L_RAWBUF,
        L_RAWLEN,
        Some(L_COLL),
        abi::mfb_return(1),
        &alloc_fail,
        vregs,
        ins,
        &mut builder.relocations,
    );
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_pub_loop"),
        &format!("{symbol}_pub_done"),
        L_RAWBUF,
        L_POINTLEN,
        Some(L_PUBCOLL),
        abi::mfb_return(1),
        &alloc_fail,
        vregs,
        ins,
        &mut builder.relocations,
    );

    // Free the pkey, wipe the SEC1/raw scratch (holds the secret scalar).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::load_u64(v9, abi::stack_pointer(), L_FREEPKEY),
        abi::branch_link_register(v9),
    ]);
    emit_zero_guarded(symbol, L_SEC1PTR, Some(L_SEC1LEN), 0, "sec1S", vregs, ins);
    emit_zero_guarded(symbol, L_RAWBUF, Some(L_RAWLEN), 0, "rawS", vregs, ins);

    let scratch = RecordBuildScratch {
        size: L_RSIZE,
        result: L_RRESULT,
        cursor: L_RCURSOR,
        block_size: L_RBLOCK,
    };
    emit_build_inlined_record(
        symbol,
        "kp",
        &ParameterType::named("KeyPair"),
        &builder.type_model,
        &[L_COLL, L_PUBCOLL],
        &scratch,
        RESULT_VALUE_REGISTER,
        &alloc_fail,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    // bug-625: the `KeyPair` holds byte copies of both lists, so free them (wiping the
    // private key), keeping the record pointer in a frame slot across the frees.
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        L_RRESULT,
    ));
    emit_free_byte_list_guarded(
        symbol,
        "lokpriv",
        L_COLL,
        L_RAWLEN,
        true,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_byte_list_guarded(
        symbol,
        "lokpub",
        L_PUBCOLL,
        L_POINTLEN,
        false,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    // bug-625: the SEC1 / SPKI / raw scratch buffers are the helper's own; free them.
    emit_free_buffer_guarded(
        symbol,
        "oksec1",
        L_SEC1PTR,
        ScratchSize::Slot(L_SEC1LEN),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_buffer_guarded(
        symbol,
        "okspki",
        L_SPKIPTR,
        ScratchSize::Slot(L_SPKILEN),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_buffer_guarded(
        symbol,
        "okraw",
        L_RAWBUF,
        ScratchSize::Slot(L_RAWLEN),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), L_RRESULT),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    // Error exits: free pkey/eckey (null-guarded via the pre-resolved free fns),
    // wipe the SEC1 scratch, then fail.
    let cleanup = |ins: &mut Vec<CodeInstruction>,
                   rel: &mut Vec<CodeRelocation>,
                   tag: &str,
                   v9: Reg,
                   vregs: &mut Vregs| {
        let skip_pk = format!("{symbol}_{tag}_nopk");
        let skip_ec = format!("{symbol}_{tag}_noec");
        ins.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&skip_pk),
            abi::load_u64(v9, abi::stack_pointer(), L_FREEPKEY),
            abi::branch_link_register(v9),
            abi::label(&skip_pk),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), L_ECKEY),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&skip_ec),
            abi::load_u64(v9, abi::stack_pointer(), L_FREEECKEY),
            abi::branch_link_register(v9),
            abi::label(&skip_ec),
        ]);
        emit_zero_guarded(
            symbol,
            L_SEC1PTR,
            Some(L_SEC1LEN),
            0,
            &format!("{tag}w"),
            vregs,
            ins,
        );
        // bug-625: the raw buffer holds the scalar too; wipe it, then free all three.
        emit_zero_guarded(
            symbol,
            L_RAWBUF,
            Some(L_RAWLEN),
            0,
            &format!("{tag}rw"),
            vregs,
            ins,
        );
        emit_free_buffer_guarded(
            symbol,
            &format!("{tag}sec1"),
            L_SEC1PTR,
            ScratchSize::Slot(L_SEC1LEN),
            ins,
            rel,
        );
        emit_free_buffer_guarded(
            symbol,
            &format!("{tag}spki"),
            L_SPKIPTR,
            ScratchSize::Slot(L_SPKILEN),
            ins,
            rel,
        );
        emit_free_buffer_guarded(
            symbol,
            &format!("{tag}raw"),
            L_RAWBUF,
            ScratchSize::Slot(L_RAWLEN),
            ins,
            rel,
        );
    };
    builder.instructions.push(abi::label(&load_fail));
    cleanup(
        &mut builder.instructions,
        &mut builder.relocations,
        "lf",
        v9,
        vregs,
    );
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&gen_fail));
    cleanup(
        &mut builder.instructions,
        &mut builder.relocations,
        "gf",
        v9,
        vregs,
    );
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
    cleanup(
        &mut builder.instructions,
        &mut builder.relocations,
        "af",
        v9,
        vregs,
    );
    // bug-625: a list built before the failure is freed too.
    emit_free_byte_list_guarded(
        symbol,
        "lafpriv",
        L_COLL,
        L_RAWLEN,
        true,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_byte_list_guarded(
        symbol,
        "lafpub",
        L_PUBCOLL,
        L_POINTLEN,
        false,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    Ok(())
}
