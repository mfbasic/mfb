//! plan-176-S: `crypto::generate`'s macos lowering. The curve tables and the dispatching
//! lowering live in `builtins/crypto/func_generate/mod.rs`.

use super::*;

// Stack frame layout for the macOS body (sp-relative; reserved by
// `finalize_vreg_body` via `stack_size`).
const SEC: usize = 0;

const CF: usize = 8;

const FN: usize = 16;

const RELEASE: usize = 24;

pub(super) const NUMVAL: usize = 32; // CFNumber int value (the key size in bits)

const NUM: usize = 40;

const DICT: usize = 48;

const KEY: usize = 56;

const DATA: usize = 64;

const KEYS: usize = 72; // keys[0]=72, keys[1]=80

const VALS: usize = 88; // vals[0]=88, vals[1]=96

const KEYCB: usize = 104;

const VALCB: usize = 112;

const BYTEPTR: usize = 120;

const BYTELEN: usize = 128;

const COLL: usize = 136; // private-key List OF Byte

const SCRATCH: usize = 144;

const PUBCOLL: usize = 152; // public-key List OF Byte

pub(super) const PUBLEN: usize = 160; // runtime SEC1 point length

const RSIZE: usize = 168;

const RRESULT: usize = 176;

const RCURSOR: usize = 184;

const RBLOCK: usize = 192;

/// Emit the macOS SecKey EC key-pair generation for the runtime curve selected into
/// the `NUMVAL` (key size in bits) and `PUBLEN` (SEC1 point length) slots. Success
/// leaves the `KeyPair` in the fallible result registers and jumps `done`; failure
/// paths release the CF objects and `emit_fail` to `done`. Self-contained: emits its
/// own fallible return, so the wrapper adds no epilogue.
pub(super) fn emit_macos_ec(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    symbol: &str,
    v9: impl Into<Reg>,
    vregs: &mut Vregs,
    done: &str,
) -> Result<(), String> {
    let v9: Reg = v9.into();
    let imports = ctx.platform_imports;
    let platform = ctx.platform;
    let load_fail = format!("{symbol}_load_fail");
    let gen_fail = format!("{symbol}_gen_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");

    // Zero the CF object slots so the error-exit cleanup can null-guard each release.
    builder.instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), NUM),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DICT),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), KEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), COLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PUBCOLL),
    ]);

    gen_cert::dlopen_one(
        symbol,
        gen_cert::SECPATH_SYMBOL,
        SEC,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    gen_cert::dlopen_one(
        symbol,
        gen_cert::CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFRelease",
        RELEASE,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;

    // CFNumber for the key size (already staged in NUMVAL by the caller).
    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFNumberCreate",
        FN,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::c_arg(1), "Integer", gen_cert::CF_NUMBER_INT_TYPE),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), NUMVAL),
    ]);
    gen_cert::call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), NUM),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);

    // Attributes dict { kSecAttrKeyType: EC, kSecAttrKeySizeInBits: <number> }.
    gen_cert::load_cf_const(
        symbol,
        SEC,
        "kSecAttrKeyType",
        KEYS,
        SCRATCH,
        &load_fail,
        imports,
        platform,
        v9,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    gen_cert::load_cf_const(
        symbol,
        SEC,
        "kSecAttrKeySizeInBits",
        KEYS + 8,
        SCRATCH,
        &load_fail,
        imports,
        platform,
        v9,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    gen_cert::load_cf_const(
        symbol,
        SEC,
        "kSecAttrKeyTypeECSECPrimeRandom",
        VALS,
        SCRATCH,
        &load_fail,
        imports,
        platform,
        v9,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::load_u64(v9, abi::stack_pointer(), NUM),
        abi::store_u64(v9, abi::stack_pointer(), VALS + 8),
    ]);
    gen_cert::dlsym_into(
        symbol,
        CF,
        "kCFTypeDictionaryKeyCallBacks",
        KEYCB,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    gen_cert::dlsym_into(
        symbol,
        CF,
        "kCFTypeDictionaryValueCallBacks",
        VALCB,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFDictionaryCreate",
        FN,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), KEYS),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), VALS),
        abi::move_immediate(abi::c_arg(3), "Integer", "2"),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), KEYCB),
        abi::load_u64(abi::c_arg(5), abi::stack_pointer(), VALCB),
    ]);
    gen_cert::call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DICT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // key = SecKeyCreateRandomKey(dict, NULL)
    gen_cert::dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateRandomKey",
        FN,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DICT),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // data = SecKeyCopyExternalRepresentation(key, NULL) -> 0x04||X||Y||K
    gen_cert::dlsym_into(
        symbol,
        SEC,
        "SecKeyCopyExternalRepresentation",
        FN,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DATA),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // raw = CFDataGetBytePtr(data); len = CFDataGetLength(data); private = List OF Byte.
    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFDataGetBytePtr",
        FN,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        DATA,
    ));
    gen_cert::call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        BYTEPTR,
    ));
    gen_cert::dlsym_into(
        symbol,
        CF,
        "CFDataGetLength",
        FN,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        DATA,
    ));
    gen_cert::call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        BYTELEN,
    ));
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_priv_loop"),
        &format!("{symbol}_priv_done"),
        BYTEPTR,
        BYTELEN,
        Some(COLL),
        abi::mfb_return(1),
        &alloc_fail,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );

    // public = first PUBLEN bytes (0x04||X||Y), sliced while BYTEPTR still aliases
    // the CFData (before its release). PUBLEN was staged by the caller.
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_pub_loop"),
        &format!("{symbol}_pub_done"),
        BYTEPTR,
        PUBLEN,
        Some(PUBCOLL),
        abi::mfb_return(1),
        &alloc_fail,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );

    // Release the CF objects, then assemble the KeyPair record last (the releases
    // clobber the caller-saved result registers).
    gen_cert::cf_release(RELEASE, NUM, v9, &mut builder.instructions);
    gen_cert::cf_release(RELEASE, DICT, v9, &mut builder.instructions);
    gen_cert::cf_release(RELEASE, KEY, v9, &mut builder.instructions);
    gen_cert::cf_release(RELEASE, DATA, v9, &mut builder.instructions);

    let scratch = RecordBuildScratch {
        size: RSIZE,
        result: RRESULT,
        cursor: RCURSOR,
        block_size: RBLOCK,
    };
    emit_build_inlined_record(
        symbol,
        "kp",
        &ParameterType::named("KeyPair"),
        &builder.type_model,
        &[COLL, PUBCOLL],
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
        RRESULT,
    ));
    emit_free_byte_list_guarded(
        symbol,
        "okpriv",
        COLL,
        BYTELEN,
        true,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_byte_list_guarded(
        symbol,
        "okpub",
        PUBCOLL,
        PUBLEN,
        false,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RRESULT),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    // Error exits: release the CF objects (null-guarded) the success path released.
    let cleanup = |ins: &mut Vec<CodeInstruction>, tag: &str| {
        gen_cert::cf_release_guarded(symbol, RELEASE, NUM, &format!("{tag}n"), v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, DICT, &format!("{tag}d"), v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, KEY, &format!("{tag}k"), v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, DATA, &format!("{tag}a"), v9, ins);
    };
    builder.instructions.push(abi::label(&load_fail));
    cleanup(&mut builder.instructions, "lf");
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&gen_fail));
    cleanup(&mut builder.instructions, "gf");
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
    cleanup(&mut builder.instructions, "af");
    // bug-625: a list built before the failure is freed too.
    emit_free_byte_list_guarded(
        symbol,
        "afpriv",
        COLL,
        BYTELEN,
        true,
        vregs,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    emit_free_byte_list_guarded(
        symbol,
        "afpub",
        PUBCOLL,
        PUBLEN,
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
