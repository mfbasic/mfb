//! `crypto::generate(type)` — a clean-room `AbiFunction` key-pair generator.
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`/
//! `X25519`), this member's `Body::abi_function` body branches on the enum ordinal.
//! For the three NIST-EC curves it generates the key pair **from scratch** here — it
//! does not call into `crypto/native/*` — reproducing the platform key sequence
//! (macOS `SecKey`, Linux OpenSSL `EVP_PKEY`, Windows CNG) self-contained on each
//! target, driving the general marshallers for the `List OF Byte`/`KeyPair` output.
//! `Ed25519` dispatches to [`super::helper_generate_ed25519`] and `X25519` to
//! [`super::helper_generate_x25519`] — both pure MFBASIC software cores.
//!
//! All three platforms are implemented and per-platform verified (macOS host, Linux
//! aarch64, Windows x86-64).

use super::gen_cert::{self, CopyLen};
use super::{Body, Implementation, Parameter, ParameterType, RegistryFunction};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::{
    emit_build_byte_list, emit_build_inlined_record, emit_zero_guarded, RecordBuildScratch,
};
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use std::collections::HashMap;

// The `Certificate` ordinals, framework/library paths, dlsym-name data objects, and
// the shared platform seam (dlopen/dlsym/CoreFoundation/BCrypt) live in
// [`super::gen_cert`]. This file keeps only the `crypto::generate`-specific per-curve
// geometry and the three platform key-generation sequences.

// ---------------------------------------------------------------------------
// Linux OpenSSL per-curve geometry (ordinal-indexed): (curve-name, nid, point_len,
// field_len, sec1_scalar_off, spki_prefix_len).
// ---------------------------------------------------------------------------
struct OsslCurve {
    name: &'static str,
    nid: &'static str,
    point_len: usize,
    field_len: usize,
    sec1_scalar_off: usize,
    spki_prefix_len: usize,
}
const OSSL_CURVES: &[OsslCurve] = &[
    OsslCurve {
        name: "P-256",
        nid: "415",
        point_len: 65,
        field_len: 32,
        sec1_scalar_off: 7,
        spki_prefix_len: 26,
    },
    OsslCurve {
        name: "P-384",
        nid: "715",
        point_len: 97,
        field_len: 48,
        sec1_scalar_off: 8,
        spki_prefix_len: 23,
    },
    OsslCurve {
        name: "P-521",
        nid: "716",
        point_len: 133,
        field_len: 66,
        sec1_scalar_off: 8,
        spki_prefix_len: 25,
    },
];

// ---------------------------------------------------------------------------
// Windows CNG per-curve parameters (ordinal-indexed): (algo-id, field_len). The raw
// (`1+3·field`) and public (`1+2·field`) lengths are derived at runtime.
// ---------------------------------------------------------------------------
struct WinCurve {
    algo: &'static str,
    field_len: usize,
    bits: &'static str,
}
const WIN_CURVES: &[WinCurve] = &[
    WinCurve {
        algo: "ECDSA_P256",
        field_len: 32,
        bits: "256",
    },
    WinCurve {
        algo: "ECDSA_P384",
        field_len: 48,
        bits: "384",
    },
    WinCurve {
        algo: "ECDSA_P521",
        field_len: 66,
        bits: "521",
    },
];

// Stack frame layout for the macOS body (sp-relative; reserved by
// `finalize_vreg_body` via `stack_size`).
const SEC: usize = 0;
const CF: usize = 8;
const FN: usize = 16;
const RELEASE: usize = 24;
const NUMVAL: usize = 32; // CFNumber int value (the key size in bits)
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
const PUBLEN: usize = 160; // runtime SEC1 point length
const RSIZE: usize = 168;
const RRESULT: usize = 176;
const RCURSOR: usize = 184;
const RBLOCK: usize = 192;
const LOCAL_SIZE: usize = 208;

/// Emit the macOS SecKey EC key-pair generation for the runtime curve selected into
/// the `NUMVAL` (key size in bits) and `PUBLEN` (SEC1 point length) slots. Success
/// leaves the `KeyPair` in the fallible result registers and jumps `done`; failure
/// paths release the CF objects and `emit_fail` to `done`. Self-contained: emits its
/// own fallible return, so the wrapper adds no epilogue.
fn emit_macos_ec(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    symbol: &str,
    v9: &str,
    done: &str,
) -> Result<(), String> {
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
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
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
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    Ok(())
}

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
const L_NID: usize = 160;
const L_POINTLEN: usize = 168;
const L_FIELDLEN: usize = 176;
const L_SEC1OFF: usize = 184;
const L_SPKIPREFIX: usize = 192;
const L_NAMEPTR: usize = 200;

/// Emit the Linux OpenSSL EC keygen for the runtime curve params staged in the
/// `L_NID`/`L_POINTLEN`/`L_FIELDLEN`/`L_SEC1OFF`/`L_SPKIPREFIX`/`L_NAMEPTR` slots.
/// Self-managed fallible ABI; jumps `done`.
#[allow(clippy::too_many_arguments)]
fn emit_linux_ec(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    symbol: &str,
    v9: &str,
    v10: &str,
    v11: &str,
    v12: &str,
    v13: &str,
    done: &str,
) -> Result<(), String> {
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
        ins,
        &mut builder.relocations,
    );

    // Free the pkey, wipe the SEC1/raw scratch (holds the secret scalar).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::load_u64(v9, abi::stack_pointer(), L_FREEPKEY),
        abi::branch_link_register(v9),
    ]);
    emit_zero_guarded(symbol, L_SEC1PTR, Some(L_SEC1LEN), 0, "sec1S", ins);
    emit_zero_guarded(symbol, L_RAWBUF, Some(L_RAWLEN), 0, "rawS", ins);

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
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    // Error exits: free pkey/eckey (null-guarded via the pre-resolved free fns),
    // wipe the SEC1 scratch, then fail.
    let cleanup = |ins: &mut Vec<CodeInstruction>, tag: &str, v9: &str| {
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
            ins,
        );
    };
    builder.instructions.push(abi::label(&load_fail));
    cleanup(&mut builder.instructions, "lf", v9);
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&gen_fail));
    cleanup(&mut builder.instructions, "gf", v9);
    emit_fail(
        symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
    cleanup(&mut builder.instructions, "af", v9);
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        done,
    );
    Ok(())
}

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
const W_FIELD: usize = 104;
const W_ALGOPTR: usize = 112;
const W_BITS: usize = 120;

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
fn emit_windows_ec(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    symbol: &str,
    v9: &str,
    v10: &str,
    v11: &str,
    v12: &str,
    v13: &str,
    done: &str,
) -> Result<(), String> {
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
    emit_zero_guarded(symbol, W_BLOB, None, gen_cert::BLOBCAP, "wblobz", ins);

    emit_build_byte_list(
        symbol,
        &format!("{symbol}_wpriv_loop"),
        &format!("{symbol}_wpriv_done"),
        W_RAW,
        W_RAWLEN,
        Some(W_COLL),
        abi::mfb_return(1),
        &alloc_fail,
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
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    builder.instructions.push(abi::label(&fail));
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

/// The `AbiFunction` body for `crypto::generate`. `args[0]` is the `Certificate`
/// ordinal (in the first argument register). Self-managed fallible ABI, so it
/// returns the `void` sentinel — the wrapper adds no epilogue.
pub(crate) fn lower_generate(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();

    // Reserve the body's sp-relative scratch (base 0) so `finalize_vreg_body` sizes
    // it, and mint the single shared scratch vreg (%v9); the general marshallers use
    // %v10.. above it.
    builder.allocate_stack_object("crypto_generate_scratch", LOCAL_SIZE);
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();

    let done = format!("{symbol}_done");
    let ed25519 = format!("{symbol}_ed25519");
    let x25519 = format!("{symbol}_x25519");
    let x448 = format!("{symbol}_x448");
    let ed448 = format!("{symbol}_ed448");

    match ctx.platform.family() {
        PlatformFamily::MacOS => {
            // Select the runtime curve params (key size in bits -> NUMVAL, SEC1 point
            // length -> PUBLEN) from the ordinal, then run the one SecKey sequence.
            let p384 = format!("{symbol}_p384");
            let p521 = format!("{symbol}_p521");
            let have = format!("{symbol}_have_params");
            let set = |ins: &mut Vec<CodeInstruction>, v9: &str, bits: &str, plen: &str| {
                ins.extend([
                    abi::move_immediate(v9, "Integer", bits),
                    abi::store_u64(v9, abi::stack_pointer(), NUMVAL),
                    abi::move_immediate(v9, "Integer", plen),
                    abi::store_u64(v9, abi::stack_pointer(), PUBLEN),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X25519),
                abi::branch_eq(&x25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X448),
                abi::branch_eq(&x448),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            set(&mut builder.instructions, &v9, "256", "65");
            builder.instructions.push(abi::label(&p384));
            set(&mut builder.instructions, &v9, "384", "97");
            builder.instructions.push(abi::label(&p521));
            set(&mut builder.instructions, &v9, "521", "133");
            builder.instructions.push(abi::label(&have));
            emit_macos_ec(builder, ctx, &symbol, &v9, &done)?;
        }
        PlatformFamily::Linux => {
            // Select the runtime OpenSSL curve params (nid, point/field lengths, DER
            // offsets, curve-name pointer) from the ordinal, then run the sequence.
            let lp384 = format!("{symbol}_lp384");
            let lp521 = format!("{symbol}_lp521");
            let have = format!("{symbol}_lhave");
            let sym_owned = symbol.clone();
            let set = |ins: &mut Vec<CodeInstruction>,
                       rel: &mut Vec<CodeRelocation>,
                       v9: &str,
                       c: &OsslCurve| {
                ins.extend([
                    abi::move_immediate(v9, "Integer", c.nid),
                    abi::store_u64(v9, abi::stack_pointer(), L_NID),
                    abi::move_immediate(v9, "Integer", &c.point_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_POINTLEN),
                    abi::move_immediate(v9, "Integer", &c.field_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_FIELDLEN),
                    abi::move_immediate(v9, "Integer", &c.sec1_scalar_off.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_SEC1OFF),
                    abi::move_immediate(v9, "Integer", &c.spki_prefix_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_SPKIPREFIX),
                ]);
                emit_data_address(&sym_owned, v9, &gen_cert::ossl_name_sym(c.name), ins, rel);
                ins.extend([
                    abi::store_u64(v9, abi::stack_pointer(), L_NAMEPTR),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&lp384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&lp521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X25519),
                abi::branch_eq(&x25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X448),
                abi::branch_eq(&x448),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                &v9,
                &OSSL_CURVES[0],
            );
            builder.instructions.push(abi::label(&lp384));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                &v9,
                &OSSL_CURVES[1],
            );
            builder.instructions.push(abi::label(&lp521));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                &v9,
                &OSSL_CURVES[2],
            );
            builder.instructions.push(abi::label(&have));
            emit_linux_ec(builder, ctx, &symbol, &v9, &v10, &v11, &v12, &v13, &done)?;
        }
        PlatformFamily::Windows => {
            // Select the runtime CNG curve params (field length, algorithm-id
            // pointer) from the ordinal, then run the BCrypt sequence.
            let wp384 = format!("{symbol}_wp384");
            let wp521 = format!("{symbol}_wp521");
            let have = format!("{symbol}_whave");
            let sym_owned = symbol.clone();
            let set = |ins: &mut Vec<CodeInstruction>,
                       rel: &mut Vec<CodeRelocation>,
                       v9: &str,
                       c: &WinCurve| {
                ins.extend([
                    abi::move_immediate(v9, "Integer", &c.field_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), W_FIELD),
                    abi::move_immediate(v9, "Integer", c.bits),
                    abi::store_u64(v9, abi::stack_pointer(), W_BITS),
                ]);
                emit_data_address(&sym_owned, v9, &gen_cert::win_sym(c.algo), ins, rel);
                ins.extend([
                    abi::store_u64(v9, abi::stack_pointer(), W_ALGOPTR),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&wp384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&wp521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X25519),
                abi::branch_eq(&x25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X448),
                abi::branch_eq(&x448),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                &v9,
                &WIN_CURVES[0],
            );
            builder.instructions.push(abi::label(&wp384));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                &v9,
                &WIN_CURVES[1],
            );
            builder.instructions.push(abi::label(&wp521));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                &v9,
                &WIN_CURVES[2],
            );
            builder.instructions.push(abi::label(&have));
            emit_windows_ec(builder, ctx, &symbol, &v9, &v10, &v11, &v12, &v13, &done)?;
        }
    }

    // Ed25519 dispatch (all platforms): call the software
    // `__crypto_generateEd25519` MFB helper (always emitted with the crypto
    // package — seed = randomBytes(32); pub = ed25519Public(seed)). It leaves the
    // `KeyPair` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed25519));
    let ed_symbol = crate::target::shared::nir::function_symbol("#crypto_generateEd25519");
    // Win64 mandates 32 bytes of caller-reserved shadow space around every call.
    let win64 = matches!(ctx.platform.family(), PlatformFamily::Windows);
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X25519 dispatch (all platforms): call the software `__crypto_generateX25519`
    // MFB helper (always emitted with the crypto package — scalar = clamp(randomBytes
    // (32)); pub = X25519(scalar, basepoint u=9)). It leaves the `KeyPair` in the
    // result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&x25519));
    let x_symbol = crate::target::shared::nir::function_symbol("#crypto_generateX25519");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&x_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X448 dispatch (all platforms): call the software `__crypto_generateX448` MFB
    // helper (scalar = clamp448(randomBytes(56)); pub = X448(scalar, basepoint u=5)).
    // It leaves the `KeyPair` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&x448));
    let x448_symbol = crate::target::shared::nir::function_symbol("#crypto_generateX448");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&x448_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // Ed448 dispatch (all platforms): call the software `__crypto_generateEd448`
    // MFB helper (seed = randomBytes(57); pub = ed448Public(seed)). It leaves the
    // `KeyPair` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed448));
    let ed448_symbol = crate::target::shared::nir::function_symbol("#crypto_generateEd448");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed448_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::parse("KeyPair"),
        location: Operand::from("void"),
        text: "crypto.generate".to_string(),
    })
}

const INTRO: &str = r#"Generate a fresh key pair of the requested certificate type."#;
const DESC: &str = r#"`crypto::generate(type)` draws a fresh random key pair for the curve selected by
`type` (a `crypto::Certificate`) and returns it as a `crypto::KeyPair`, whose
`privateKey` and `publicKey` fields are each a `List OF Byte`. The signing key
pairs — the three NIST prime curves (`P256`/`P384`/`P521`, FIPS 186-4 ECDSA over
SEC/NIST `secp256r1`/`secp384r1`/`secp521r1`), `Ed25519`, and `Ed448` (RFC 8032
EdDSA) — are usable with `crypto::sign(type, …)` and `crypto::verify(type, …)` for
that same `type`. `X25519` and `X448` produce Curve25519 / Curve448 ECDH key-agreement
pairs (RFC 7748) for `crypto::exchange`; they are **not** signing keys, so
`sign`/`verify` reject them with `ErrInvalidArgument`. Note that
`crypto::encrypt`/`crypto::decrypt` take **Ed25519** or **Ed448** keys (converting
them to X25519 / X448 internally, as `crypto::convert` does), so a directly
generated `X25519`/`X448` pair is the raw Diffie-Hellman building block for
`crypto::exchange` rather than a direct input to the asymmetric-encryption members.

**Encodings and sizes.** Every field is raw big-endian bytes — no PEM, no
base64, no DER wrapper on the key material itself. For the NIST curves the
`publicKey` is one **SEC1 / X9.62 uncompressed point** `0x04‖X‖Y` (each coordinate
`field` bytes), and the `privateKey` is that same uncompressed point immediately
followed by the big-endian secret scalar `d` (i.e. `0x04‖X‖Y‖d`). For `Ed25519`
the `privateKey` is the 32-byte seed and the `publicKey` is the 32-byte compressed
point; for `X25519` both are 32-byte Curve25519 values, for `X448` both are
56-byte Curve448 values (little-endian `u`-coordinate and clamped scalar), and for
`Ed448` the `privateKey` is the 57-byte seed and the `publicKey` the 57-byte
compressed edwards448 point (56-byte `y` plus the sign byte).

| `type` | Standard | Digest (sign/verify) | `publicKey` | `privateKey` |
| --- | --- | --- | --- | --- |
| `P256` | secp256r1, FIPS 186-4 | SHA-256 | 65 B (`0x04‖X‖Y`) | 97 B (`0x04‖X‖Y‖d`) |
| `P384` | secp384r1, FIPS 186-4 | SHA-384 | 97 B | 145 B |
| `P521` | secp521r1, FIPS 186-4 | SHA-512 | 133 B | 199 B |
| `Ed25519` | RFC 8032 | SHA-512 (internal) | 32 B | 32 B (seed) |
| `X25519` | RFC 7748 | — (not a signing key) | 32 B | 32 B |
| `X448` | RFC 7748 | — (not a signing key) | 56 B | 56 B |
| `Ed448` | RFC 8032 | SHAKE256 (internal) | 57 B | 57 B (seed) |

**Security.** The `privateKey` is secret key material — keep it confidential and
never transmit or log it; only the `publicKey` is safe to share. Randomness comes
from the platform CSPRNG (for the NIST curves) or `crypto::randomBytes` (for the
software curves).

**Implementation.** The three NIST curves are generated through the host
platform's key API, reproduced clean-room in this member (no third-party crypto is
bundled): on **macOS** via Security.framework `SecKey` (`SecKeyCreateRandomKey`,
EC key type, exported with `SecKeyCopyExternalRepresentation`); on **Linux** via
OpenSSL `libcrypto` (`EVP_EC_gen` on 3.x, else `EC_KEY_new_by_curve_name` +
`EC_KEY_generate_key`, serialized through `i2d_PrivateKey`/`i2d_PUBKEY`); on
**Windows** via CNG `bcrypt.dll` (`BCryptGenerateKeyPair` +
`BCryptFinalizeKeyPair` + `BCryptExportKey`, algorithms `ECDSA_P256/384/521`,
`ECCPRIVATEBLOB`). `Ed25519`, `X25519`, `X448`, and `Ed448` are a pure in-process
MFBASIC software core (over the `bits` package) with **no platform library**, so they are
byte-identical on every OS. Across platforms the encodings are wire-compatible: a
key made on one OS is accepted by `sign`/`verify` on the others."#;
const EX: &str = r#"Generate an ECDSA P-256 pair and use both halves:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET kp AS crypto::KeyPair = crypto::generate(Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::verify(Certificate.P256, kp.publicKey, msg, sig)
END SUB
```

Ed25519 keys are byte-identical on every platform:

```
IMPORT crypto

SUB main()
  LET kp AS crypto::KeyPair = crypto::generate(Certificate.Ed25519)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "type",
                desc: "The certificate/key type to generate.",
                aliases: &[],
                ty: ParameterType::named("Certificate"),
                default: crate::codegen::registry::DefaultValue::None,
            }],
            return_type: ParameterType::named("KeyPair"),
            errors: vec!["ErrOutOfMemory", "ErrUnknown"],
            body: Body::abi_function(lower_generate),
        }],
    });
}
