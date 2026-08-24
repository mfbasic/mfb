//! `crypto::sign(type, privateKey, message)` — a clean-room `AbiFunction` signer.
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`),
//! this member's `Body::abi_function` body branches on the enum ordinal and, for
//! the three NIST-EC curves, signs the message **from scratch** here — it does not
//! call into `crypto/native/*` (the migrated per-curve `p{256,384,521}Sign`). It is
//! *modeled on* those (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG) but reproduces
//! the platform sequence in this file, self-contained, driving the general
//! marshallers for the `List OF Byte` inputs/output. `Ed25519` dispatches to the
//! software [`super::helper_ed25519_sign`] helper (`__crypto_ed25519Sign`).
//!
//! Structure mirrors [`super::func_generate`]: the `Certificate` ordinal consts, a
//! per-platform seam with self-contained clean-room helpers, a `data_objects(family)`
//! driver-gate, and the `Ed25519` branch that calls the always-emitted MFB helper.
//! The one deviation: because each NIST curve differs in its algorithm/DER details,
//! the ordinal branch dispatches to a full per-curve *compile-time* sequence (each a
//! faithful reproduction of the matching `p*Sign` reference), rather than staging
//! per-curve params into one runtime-parametric sequence.
//!
//! Output encoding is identical to the per-curve signers: ECDSA is ASN.1 DER
//! `Ecdsa-Sig-Value`; Ed25519 is the 64-byte raw `R‖S`.

use super::gen_cert::{self, CopyLen, Sc};
use super::{bytes, Body, Implementation, Parameter, ParameterType, RegistryFunction};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::{
    emit_build_byte_list, emit_read_byte_list, emit_zero_guarded,
};
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use std::collections::HashMap;

// The `Certificate` ordinals, framework/library paths, dlsym-name and PKCS#8-template
// data objects, and the shared platform seam (dlopen/dlsym/CoreFoundation/BCrypt) live
// in [`super::gen_cert`]. This file keeps only the `crypto::sign`-specific per-curve
// selector and the three platform signing sequences.

/// The NIST curves this signer covers. A compile-time selector: the ordinal branch
/// in `lower_sign` picks the curve, then emits its full sequence.
#[derive(Clone, Copy)]
enum SignCurve {
    P256,
    P384,
    P521,
}

impl SignCurve {
    fn field_len(self) -> usize {
        match self {
            SignCurve::P256 => 32,
            SignCurve::P384 => 48,
            SignCurve::P521 => 66,
        }
    }
    fn point_len(self) -> usize {
        match self {
            SignCurve::P256 => 65,
            SignCurve::P384 => 97,
            SignCurve::P521 => 133,
        }
    }
    /// The macOS `SecKeyAlgorithm` constant (a CFString) for ECDSA over a message.
    fn macos_algorithm(self) -> &'static str {
        match self {
            SignCurve::P256 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
            SignCurve::P384 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
            SignCurve::P521 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
        }
    }
    /// OpenSSL PKCS#8 template + splice geometry.
    fn pkcs8_len(self) -> usize {
        match self {
            SignCurve::P256 => 138,
            SignCurve::P384 => 185,
            SignCurve::P521 => 241,
        }
    }
    fn p8_scalar_off(self) -> usize {
        match self {
            SignCurve::P256 => 36,
            SignCurve::P384 => 35,
            SignCurve::P521 => 35,
        }
    }
    fn p8_point_off(self) -> usize {
        match self {
            SignCurve::P256 => 73,
            SignCurve::P384 => 88,
            SignCurve::P521 => 108,
        }
    }
    fn name(self) -> &'static str {
        match self {
            SignCurve::P256 => "P-256",
            SignCurve::P384 => "P-384",
            SignCurve::P521 => "P-521",
        }
    }
    fn digest(self) -> &'static str {
        match self {
            SignCurve::P256 => "EVP_sha256",
            SignCurve::P384 => "EVP_sha384",
            SignCurve::P521 => "EVP_sha512",
        }
    }
    /// Windows CNG identifiers + magic.
    fn algo_id(self) -> &'static str {
        match self {
            SignCurve::P256 => "ECDSA_P256",
            SignCurve::P384 => "ECDSA_P384",
            SignCurve::P521 => "ECDSA_P521",
        }
    }
    fn hash_id(self) -> &'static str {
        match self {
            SignCurve::P256 => "SHA256",
            SignCurve::P384 => "SHA384",
            SignCurve::P521 => "SHA512",
        }
    }
    fn hash_len(self) -> usize {
        match self {
            SignCurve::P256 => 32,
            SignCurve::P384 => 48,
            SignCurve::P521 => 64,
        }
    }
    fn priv_magic(self) -> &'static str {
        match self {
            SignCurve::P256 => "844317509", // 0x32534345 'ECS2'
            SignCurve::P384 => "877871941", // 0x34534345 'ECS4'
            SignCurve::P521 => "911426373", // 0x36534345 'ECS6'
        }
    }
}

/// Emit the macOS SecKey ECDSA sign sequence for `curve`, self-contained. The two
/// argument collection pointers arrive in `priv_op`/`msg_op`. Success leaves the DER
/// signature `List OF Byte` in the result registers and branches `done`; failure
/// paths release the CF objects and `emit_fail` to `done`.
#[allow(clippy::too_many_arguments)]
fn emit_macos_sign(
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
        ins,
    );

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    let cleanup = |ins: &mut Vec<CodeInstruction>, ctag: &str| {
        gen_cert::cf_release_guarded(symbol, RELEASE, PRIVDATA, &format!("{ctag}p"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, MSGDATA, &format!("{ctag}m"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, DICT, &format!("{ctag}d"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, KEY, &format!("{ctag}k"), &v9, ins);
        gen_cert::cf_release_guarded(symbol, RELEASE, SIGDATA, &format!("{ctag}s"), &v9, ins);
        emit_zero_guarded(symbol, PRIVBUF, Some(PRIVLEN), 0, &format!("{ctag}z"), ins);
    };
    ins.push(abi::label(&load_fail));
    cleanup(ins, &format!("{tag}lf"));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&sign_fail));
    cleanup(ins, &format!("{tag}sf"));
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&invalid_fail));
    cleanup(ins, &format!("{tag}iv"));
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
    ins.push(abi::label(&alloc_fail));
    cleanup(ins, &format!("{tag}af"));
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    Ok(())
}

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
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
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
fn emit_linux_sign(
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
        ins,
        rel,
    );
    // priv len must be the SEC1 private (point ‖ scalar).
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), PRIVLEN),
        abi::compare_immediate(&v9, (point_len + field_len).to_string()),
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
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
    ins.push(abi::store_u64(
        abi::return_register(),
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
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MD,
    ));

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
        abi::compare_immediate(abi::return_register(), "1"),
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
        abi::compare_immediate(abi::return_register(), "1"),
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
        abi::compare_immediate(abi::return_register(), "1"),
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
        ins,
    );
    emit_zero_guarded(symbol, DERBUF, None, pkcs8_len, &format!("{tag}derS"), ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
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
        emit_zero_guarded(symbol, PRIVBUF, Some(PRIVLEN), 0, &format!("{ctag}pz"), ins);
        emit_zero_guarded(symbol, DERBUF, None, pkcs8_len, &format!("{ctag}dz"), ins);
        Ok(())
    };
    ins.push(abi::label(&load_fail));
    cleanup(ins, rel, &format!("{tag}lf"))?;
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    ins.push(abi::label(&sign_fail));
    cleanup(ins, rel, &format!("{tag}sf"))?;
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

// ===========================================================================
// Windows CNG clean-room emit helpers (reproduced; not calls into native).
// ===========================================================================

/// Encode one big-endian `field`-wide integer at `[src]` as an ASN.1 INTEGER into
/// `[dst]`, advancing `dst` past the written bytes.
fn win_der_encode_int(
    sc: &Sc,
    src: &str,
    dst: &str,
    field: usize,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
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
fn emit_windows_sign(
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
        ins,
        rel,
    );
    // priv len must be the SEC1 private (point ‖ scalar).
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), PRIVLEN),
        abi::compare_immediate(&sc.v9, priv_raw.to_string()),
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
        ins,
    );
    emit_zero_guarded(symbol, BLOB, None, blobcap, &format!("{tag}blobz"), ins);
    ins.push(abi::branch(done));
    Ok(())
}

/// The body's sp-relative scratch size — the max over the three platform layouts
/// (macOS reaches CONST_SCRATCH=200 → 208; Linux 128 → 144; Windows 136 + 64 = 200).
const LOCAL_SIZE: usize = 208;

/// The `AbiFunction` body for `crypto::sign`. `args[0]` is the `Certificate` ordinal,
/// `args[1]` the private-key collection pointer, `args[2]` the message collection
/// pointer — each in its argument register. Self-managed fallible ABI, so it returns
/// the `void` sentinel (the wrapper adds no epilogue).
pub(crate) fn lower_sign(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    let priv_op = args[1].location.clone();
    let msg_op = args[2].location.clone();

    builder.allocate_stack_object("crypto_sign_scratch", LOCAL_SIZE);

    let done = format!("{symbol}_done");
    let ed25519 = format!("{symbol}_ed25519");
    let x25519_reject = format!("{symbol}_x25519_reject");
    let imports = ctx.platform_imports;
    let platform = ctx.platform;

    // X25519 is a key-agreement (ECDH) key, not a signing key: reject it up front
    // rather than fall through to the P-256 sequence.
    builder.instructions.extend([
        abi::compare_immediate(&ord, gen_cert::ORD_X25519),
        abi::branch_eq(&x25519_reject),
    ]);

    match platform.family() {
        PlatformFamily::MacOS => {
            let p384 = format!("{symbol}_mp384");
            let p521 = format!("{symbol}_mp521");
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
            ]);
            // P-256 (ordinal 0) falls through here.
            emit_macos_sign(
                SignCurve::P256,
                &symbol,
                "mp256",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            emit_macos_sign(
                SignCurve::P384,
                &symbol,
                "mp384",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            emit_macos_sign(
                SignCurve::P521,
                &symbol,
                "mp521",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }
        PlatformFamily::Linux => {
            let p384 = format!("{symbol}_lp384");
            let p521 = format!("{symbol}_lp521");
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
            ]);
            emit_linux_sign(
                SignCurve::P256,
                &symbol,
                "lp256",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            emit_linux_sign(
                SignCurve::P384,
                &symbol,
                "lp384",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            emit_linux_sign(
                SignCurve::P521,
                &symbol,
                "lp521",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }
        PlatformFamily::Windows => {
            let p384 = format!("{symbol}_wp384");
            let p521 = format!("{symbol}_wp521");
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
            ]);
            emit_windows_sign(
                SignCurve::P256,
                &symbol,
                "wp256",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            emit_windows_sign(
                SignCurve::P384,
                &symbol,
                "wp384",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            emit_windows_sign(
                SignCurve::P521,
                &symbol,
                "wp521",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }
    }

    // Ed25519 dispatch (all platforms): route the (privateKey, message) pair into the
    // callee argument registers and call the software `__crypto_ed25519Sign` MFB
    // helper (always emitted with the crypto package). It leaves the 64-byte raw
    // signature `List OF Byte` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed25519));
    builder.instructions.extend([
        abi::move_register(abi::argument_register(0)?, priv_op),
        abi::move_register(abi::argument_register(1)?, msg_op),
    ]);
    let ed_symbol = crate::target::shared::nir::function_symbol("#crypto_ed25519Sign");
    // Win64 mandates 32 bytes of caller-reserved shadow space around every call.
    let win64 = matches!(platform.family(), PlatformFamily::Windows);
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X25519 keys cannot sign.
    builder.instructions.push(abi::label(&x25519_reject));
    emit_fail(
        &symbol,
        "ErrInvalidArgument",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        origin: None,
        type_: "List OF Byte".to_string(),
        location: Operand::from("void"),
        text: "crypto.sign".to_string(),
    })
}

const INTRO: &str = r#"Sign a message with a private key of the given certificate type."#;
const DESC: &str = r#"`crypto::sign(type, privateKey, message)` produces a digital signature over the
raw bytes of `message` using `privateKey`, for the NIST prime curve or `Ed25519`
selected by `type` (a `crypto::Certificate`), and returns it as a `List OF Byte`.
`privateKey` is the exact `privateKey` field of the `crypto::KeyPair` that
`crypto::generate(type)` returned for the same `type` — for the NIST curves the
SEC1 uncompressed point followed by the secret scalar (`0x04‖X‖Y‖d`, 97/145/199
bytes for `P256`/`P384`/`P521`), and for `Ed25519` the 32-byte seed.

For the NIST curves this is **FIPS 186-4 ECDSA** with the curve's mandated digest
(**SHA-256** for `P256`, **SHA-384** for `P384`, **SHA-512** for `P521`) over the
whole message, and the result is an **ASN.1 DER** `Ecdsa-Sig-Value` (a `SEQUENCE {
INTEGER r, INTEGER s }`, X9.62); its length is variable (roughly 70–72 bytes for
`P256`) because `r`/`s` are minimally encoded. ECDSA is randomized, so signing the
same message twice yields different (both valid) signatures. ECDSA signatures are
also **malleable**: from one valid `(r, s)` a third party can derive another
signature that verifies for the same message and key (e.g. `(r, n − s)`), and the
DER can be re-serialized — this implementation does not enforce a canonical low-S
form. Never treat a signature's bytes (or a hash of them) as a unique identifier,
and never use them for replay protection or deduplication. For `Ed25519` this is
**RFC 8032 PureEdDSA** (deterministic; the message is hashed with SHA-512
internally) and the result is the fixed **64-byte** raw `R‖S`. Either is
verifiable with `crypto::verify(type, …)` for the same `type`.

**Boundary and errors.** A `privateKey` whose length is not the exact SEC1 size
for the chosen curve, or an `Ed25519` key that is not 32 bytes, or an otherwise
malformed key the platform import rejects, raises `ErrInvalidArgument`; `message`
may be any length, including empty. `X25519` is a key-agreement key and cannot
sign — it raises `ErrInvalidArgument`. A platform-library or system failure raises
`ErrUnknown`, and an allocation failure raises `ErrOutOfMemory`. The private key
material is zeroed from the working buffers before return.

**Implementation.** ECDSA signing runs through the host platform key API,
reproduced clean-room in this member (no third-party crypto is bundled): on
**macOS** via Security.framework `SecKeyCreateWithData` + `SecKeyCreateSignature`
with `kSecKeyAlgorithmECDSASignatureMessageX962SHA256/384/512`; on **Linux** via
OpenSSL `libcrypto` `EVP_DigestSignInit` + one-shot `EVP_DigestSign`
(`EVP_sha256/384/512`), the SEC1 key spliced into a PKCS#8 template and decoded
with `d2i_AutoPrivateKey`; on **Windows** via CNG `bcrypt.dll`
`BCryptImportKeyPair` (`ECCPRIVATEBLOB`) + `BCryptHash` + `BCryptSignHash`, whose
fixed `r‖s` output is re-encoded to ASN.1 DER here. `Ed25519` is a pure in-process
MFBASIC software core (RFC 8032) with **no platform library**, so it is
byte-identical on every OS. Signatures are wire-compatible across platforms."#;
const EX: &str = r#"```
IMPORT crypto
IMPORT strings

SUB main()
  LET kp AS crypto::KeyPair = crypto::generate(Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::verify(Certificate.P256, kp.publicKey, msg, sig)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sign",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The certificate/key type of the private key.",
                    aliases: &[],
                    ty: ParameterType::named("Certificate"),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "privateKey",
                    desc: "The private key bytes (the `privateKey` field of a `crypto::KeyPair`).",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "message",
                    desc: "The message bytes to sign.",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument", "ErrOutOfMemory", "ErrUnknown"],
            body: Body::abi_function(lower_sign),
        }],
    });
}
