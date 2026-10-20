//! plan-176-S: `crypto::generate`'s windows lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_generate/mod.rs`.

use super::*;

// Windows stack layout (sp-relative).
const W_HALG: usize = 0;

const W_HKEY: usize = 8;

const W_BLOB: usize = 16;

const W_CBRES: usize = 24;

const W_RAW: usize = 32;

const W_RAWLEN: usize = 40;

const W_COLL: usize = 48;

const W_PUBCOLL: usize = 56;

const W_POINTLEN: usize = 64;

const W_RSIZE: usize = 72;

const W_RRESULT: usize = 80;

const W_RCURSOR: usize = 88;

const W_RBLOCK: usize = 96;

pub(super) const W_FIELD: usize = 104;

pub(super) const W_ALGOPTR: usize = 112;

pub(super) const W_BITS: usize = 120;

/// Destroy `hKey` (`W_HKEY`) and close `hAlg` (`W_HALG`), each null-guarded, then
/// null the slots so the shared error labels can reuse this idempotently.
fn win_cleanup(
    symbol: &str,
    tag: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let no_key = format!("{symbol}_wnokey_{tag}");
    let no_alg = format!("{symbol}_wnoalg_{tag}");
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_key),
    ]);
    gen_cert::bcrypt_call(symbol, "BCryptDestroyKey", 1, imports, platform, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_HKEY),
        abi::label(&no_key),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HALG),
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
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_HALG),
        abi::label(&no_alg),
    ]);
    Ok(())
}

/// Emit the Windows CNG EC keygen for the runtime curve params staged in
/// `W_FIELD` (field length) and `W_ALGOPTR` (the `LPCWSTR` algorithm id).
/// Self-managed fallible ABI; jumps `done`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_windows_ec(
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
    let fail = format!("{symbol}_wfail");
    let alloc_fail = format!("{symbol}_walloc_fail");
    let cpy = format!("{symbol}_wcpy");
    let cpy_end = format!("{symbol}_wcpyend");
    let ins = &mut builder.instructions;

    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_HKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_BLOB),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_COLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_PUBCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_RAW),
    ]);
    // raw_len = 1 + 3·field; point_len = 1 + 2·field.
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), W_FIELD),
        abi::add_registers(v10, v9, v9),
        abi::add_registers(v10, v10, v9),
        abi::add_immediate(v10, v10, 1),
        abi::store_u64(v10, abi::stack_pointer(), W_RAWLEN),
        abi::add_registers(v11, v9, v9),
        abi::add_immediate(v11, v11, 1),
        abi::store_u64(v11, abi::stack_pointer(), W_POINTLEN),
    ]);

    // BCryptOpenAlgorithmProvider(&hAlg, algo, NULL, 0)
    ins.push(abi::add_immediate(
        abi::return_register(),
        abi::stack_pointer(),
        W_HALG,
    ));
    ins.extend([
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), W_ALGOPTR),
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
        &mut builder.relocations,
    )?;
    ins.push(abi::branch_lt(&fail));

    // BCryptGenerateKeyPair(hAlg, &hKey, bits, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HALG),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), W_HKEY),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), W_BITS),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptGenerateKeyPair",
        4,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.push(abi::branch_lt(&fail));

    // BCryptFinalizeKeyPair(hKey, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptFinalizeKeyPair",
        2,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.push(abi::branch_lt(&fail));

    // blob buffer (fixed cap, align 8) and raw output buffer (raw_len, align 1).
    ins.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &gen_cert::BLOBCAP.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, ins, &mut builder.relocations, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), W_BLOB),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_RAWLEN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, &mut builder.relocations, &alloc_fail);
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        W_RAW,
    ));

    // BCryptExportKey(hKey, NULL, L"ECCPRIVATEBLOB", blob, gen_cert::BLOBCAP, &cbResult, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    emit_data_address(
        symbol,
        abi::c_arg(2),
        &gen_cert::win_sym("ECCPRIVATEBLOB"),
        ins,
        &mut builder.relocations,
    );
    ins.extend([
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), W_BLOB),
        abi::move_immediate(abi::c_arg(4), "Integer", &gen_cert::BLOBCAP.to_string()),
        abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), W_CBRES),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptExportKey",
        7,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    ins.push(abi::branch_lt(&fail));

    // raw = 0x04 ‖ (blob body X‖Y‖d). Body starts at blob+8, length = raw_len-1.
    ins.extend([
        abi::load_u64(v10, abi::stack_pointer(), W_RAW),
        abi::move_immediate(v9, "Byte", "4"),
        abi::store_u8(v9, v10, 0),
        abi::load_u64(v11, abi::stack_pointer(), W_BLOB),
        abi::add_immediate(v11, v11, 8),
        abi::add_immediate(v12, v10, 1),
        abi::load_u64(v13, abi::stack_pointer(), W_RAWLEN),
        abi::subtract_immediate(v13, v13, 1),
        abi::label(&cpy),
        abi::compare_immediate(v13, "0"),
        abi::branch_eq(&cpy_end),
        abi::load_u8(v9, v11, 0),
        abi::store_u8(v9, v12, 0),
        abi::add_immediate(v11, v11, 1),
        abi::add_immediate(v12, v12, 1),
        abi::subtract_immediate(v13, v13, 1),
        abi::branch(&cpy),
        abi::label(&cpy_end),
    ]);

    // Clean up the handles and wipe the private blob before assembling the result
    // (the cleanup calls clobber the caller-saved result registers).
    win_cleanup(
        symbol,
        "c1",
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    emit_zero_guarded(
        symbol,
        W_BLOB,
        None,
        gen_cert::BLOBCAP,
        "wblobz",
        vregs,
        ins,
    );

    emit_build_byte_list(
        symbol,
        &format!("{symbol}_wpriv_loop"),
        &format!("{symbol}_wpriv_done"),
        W_RAW,
        W_RAWLEN,
        Some(W_COLL),
        abi::mfb_return(1),
        &alloc_fail,
        vregs,
        ins,
        &mut builder.relocations,
    );
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_wpub_loop"),
        &format!("{symbol}_wpub_done"),
        W_RAW,
        W_POINTLEN,
        Some(W_PUBCOLL),
        abi::mfb_return(1),
        &alloc_fail,
        vregs,
        ins,
        &mut builder.relocations,
    );

    let scratch = RecordBuildScratch {
        size: W_RSIZE,
        result: W_RRESULT,
        cursor: W_RCURSOR,
        block_size: W_RBLOCK,
    };
    emit_build_inlined_record(
        symbol,
        "wkp",
        &ParameterType::named("KeyPair"),
        &builder.type_model,
        &[W_COLL, W_PUBCOLL],
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
        W_RRESULT,
    ));
    emit_free_byte_list_guarded(
        symbol,
        "wokpriv",
        W_COLL,
        W_RAWLEN,
        true,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_byte_list_guarded(
        symbol,
        "wokpub",
        W_PUBCOLL,
        W_POINTLEN,
        false,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    // bug-625: wipe (both hold the private scalar) and free the blob and raw scratch.
    emit_zero_guarded(
        symbol,
        W_BLOB,
        None,
        gen_cert::BLOBCAP,
        "wokblobz",
        vregs,
        &mut builder.instructions,
    );
    emit_zero_guarded(
        symbol,
        W_RAW,
        Some(W_RAWLEN),
        0,
        "wokrawz",
        vregs,
        &mut builder.instructions,
    );
    emit_free_buffer_guarded(
        symbol,
        "wokblob",
        W_BLOB,
        ScratchSize::Bytes(gen_cert::BLOBCAP),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_buffer_guarded(
        symbol,
        "wokraw",
        W_RAW,
        ScratchSize::Slot(W_RAWLEN),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), W_RRESULT),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    builder.instructions.push(abi::label(&fail));
    // bug-625: wipe (both hold the private scalar) and free the blob and raw scratch.
    emit_zero_guarded(
        symbol,
        W_BLOB,
        None,
        gen_cert::BLOBCAP,
        "wfblobz",
        vregs,
        &mut builder.instructions,
    );
    emit_zero_guarded(
        symbol,
        W_RAW,
        Some(W_RAWLEN),
        0,
        "wfrawz",
        vregs,
        &mut builder.instructions,
    );
    emit_free_buffer_guarded(
        symbol,
        "wfblob",
        W_BLOB,
        ScratchSize::Bytes(gen_cert::BLOBCAP),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_buffer_guarded(
        symbol,
        "wfraw",
        W_RAW,
        ScratchSize::Slot(W_RAWLEN),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    win_cleanup(
        symbol,
        "c2",
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
    // bug-625: wipe (both hold the private scalar) and free the blob and raw scratch.
    emit_zero_guarded(
        symbol,
        W_BLOB,
        None,
        gen_cert::BLOBCAP,
        "wafblobz",
        vregs,
        &mut builder.instructions,
    );
    emit_zero_guarded(
        symbol,
        W_RAW,
        Some(W_RAWLEN),
        0,
        "wafrawz",
        vregs,
        &mut builder.instructions,
    );
    emit_free_buffer_guarded(
        symbol,
        "wafblob",
        W_BLOB,
        ScratchSize::Bytes(gen_cert::BLOBCAP),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_buffer_guarded(
        symbol,
        "wafraw",
        W_RAW,
        ScratchSize::Slot(W_RAWLEN),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    // bug-625: a list built before the failure is freed too.
    emit_free_byte_list_guarded(
        symbol,
        "wafpriv",
        W_COLL,
        W_RAWLEN,
        true,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_byte_list_guarded(
        symbol,
        "wafpub",
        W_PUBCOLL,
        W_POINTLEN,
        false,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    win_cleanup(
        symbol,
        "c3",
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    Ok(())
}
