//! `crypto::verify(type, publicKey, message, signature)` — a clean-room `AbiFunction`
//! signature verifier, the counterpart to [`super::func_sign`].
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`), this
//! member's `Body::abi_function` body branches on the enum ordinal and, for the three
//! NIST-EC curves, verifies the signature **from scratch** here — it does not call into
//! `crypto/native/*` (the `p{256,384,521}Verify` helpers it replaces). It is *modeled
//! on* those (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG) but reproduces the platform
//! sequence in this file, self-contained, driving the general marshallers for the
//! `List OF Byte` inputs. `Ed25519` dispatches to the software `__crypto_ed25519Verify`
//! helper.
//!
//! Semantics: a valid signature returns Boolean TRUE, an invalid/mismatched one returns
//! Boolean FALSE (NOT a raised error). A malformed public key (wrong length, or a
//! right-length off-curve key the platform import rejects) raises `ErrInvalidArgument`,
//! matching the per-curve `p*Verify`. The body self-manages its result (like
//! [`super::func_sign::lower_sign`]).
//!
//! Structure mirrors [`super::func_sign`]: the `Certificate` ordinal consts and per-
//! platform seam live in [`super::gen_cert`]; this file keeps the `crypto::verify`-
//! specific per-curve selector and the three platform verify sequences.
//!
//! Windows note (bug-446): the old `p*Verify` returned FALSE for a valid signature
//! because the CNG verify read the `BCryptVerifySignature` status from
//! `return_register()`, which on Win64 is the *aligned MFB result bank* (`rcx`, not
//! `rax`) — so it saw the stale first argument (the key handle, non-zero) instead of the
//! NTSTATUS and never matched `== 0`. This clean-room verify reads the status from the
//! C-return register (`c_return(0)` = `rax`) where the ABI actually places it.

use super::gen_cert::{self, CopyLen, Sc};
use super::{bytes, Body, Implementation, Parameter, ParameterType, RegistryFunction};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::emit_read_byte_list;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use std::collections::HashMap;

/// The NIST curves this verifier covers. A compile-time selector: the ordinal branch
/// in `lower_verify` picks the curve, then emits its full sequence.
#[derive(Clone, Copy)]
enum VerifyCurve {
    P256,
    P384,
    P521,
}

impl VerifyCurve {
    fn field_len(self) -> usize {
        match self {
            VerifyCurve::P256 => 32,
            VerifyCurve::P384 => 48,
            VerifyCurve::P521 => 66,
        }
    }
    fn point_len(self) -> usize {
        match self {
            VerifyCurve::P256 => 65,
            VerifyCurve::P384 => 97,
            VerifyCurve::P521 => 133,
        }
    }
    /// The macOS `SecKeyAlgorithm` constant (a CFString) for ECDSA over a message.
    fn macos_algorithm(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
            VerifyCurve::P384 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
            VerifyCurve::P521 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
        }
    }
    fn name(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "P-256",
            VerifyCurve::P384 => "P-384",
            VerifyCurve::P521 => "P-521",
        }
    }
    fn digest(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "EVP_sha256",
            VerifyCurve::P384 => "EVP_sha384",
            VerifyCurve::P521 => "EVP_sha512",
        }
    }
    fn algo_id(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "ECDSA_P256",
            VerifyCurve::P384 => "ECDSA_P384",
            VerifyCurve::P521 => "ECDSA_P521",
        }
    }
    fn hash_id(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "SHA256",
            VerifyCurve::P384 => "SHA384",
            VerifyCurve::P521 => "SHA512",
        }
    }
    fn hash_len(self) -> usize {
        match self {
            VerifyCurve::P256 => 32,
            VerifyCurve::P384 => 48,
            VerifyCurve::P521 => 64,
        }
    }
    fn pub_magic(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "827540293", // 0x31534345 'ECS1'
            VerifyCurve::P384 => "861094725", // 0x33534345 'ECS3'
            VerifyCurve::P521 => "894649157", // 0x35534345 'ECS5'
        }
    }
}

// ===========================================================================
// macOS SecKey clean-room verify (reproduced; not a call into native).
// ===========================================================================

/// Emit the macOS SecKey ECDSA verify sequence for `curve`, self-contained. The three
/// argument collection pointers arrive in `pub_op`/`msg_op`/`sig_op`. Success leaves the
/// Boolean verdict in the result registers and branches `done`; a malformed key raises
/// `ErrInvalidArgument`.
#[allow(clippy::too_many_arguments)]
fn emit_macos_verify(
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
        ins,
        rel,
    );
    // Reject a public key that is not exactly one uncompressed SEC1 point (parity with
    // the OpenSSL/CNG backends, which check the length explicitly).
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), PUBLEN),
        abi::compare_immediate(&v9, curve.point_len().to_string()),
        abi::branch_ne(&invalid_fail),
    ]);
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
    emit_read_byte_list(
        symbol,
        &format!("{tag}sig"),
        SIGCOLL,
        SIGBUF,
        SIGLEN,
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

/// Emit the Linux OpenSSL ECDSA verify sequence for `curve`, self-contained. The public
/// key is prefixed with a fixed SPKI DER header, decoded via `d2i_PUBKEY`, and checked
/// with one-shot `EVP_DigestVerify`. Self-managed fallible ABI.
#[allow(clippy::too_many_arguments)]
fn emit_linux_verify(
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
    emit_read_byte_list(
        symbol,
        &format!("{tag}sig"),
        SIGCOLL,
        SIGBUF,
        SIGLEN,
        &alloc_fail,
        ins,
        rel,
    );
    // pub len must be exactly one uncompressed SEC1 point.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), PUBLEN),
        abi::compare_immediate(&v9, point_len.to_string()),
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
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
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
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
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

// ===========================================================================
// Windows CNG clean-room verify (reproduced; not a call into native).
// ===========================================================================

/// Decode one ASN.1 INTEGER at `[body]` into the big-endian `field`-wide slot at `[dst]`
/// (left-padded with zeros). Advances `body` past the INTEGER. Branches to `fail` on a
/// malformed tag / oversized value, and to `bounds_fail` on any length that would read
/// past the untrusted signature buffer (`[sigbuf_off]`..+`[siglen_off]`). Scratch:
/// `sc.v8`..`sc.v14`.
#[allow(clippy::too_many_arguments)]
fn win_der_decode_int(
    sc: &Sc,
    body: &str,
    dst: &str,
    field: usize,
    tag: &str,
    fail: &str,
    sigbuf_off: usize,
    siglen_off: usize,
    bounds_fail: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let no_pad = format!("{tag}_dnp");
    ins.extend([
        // Bound the 2-byte tag+length header against the signature buffer end.
        abi::load_u64(&sc.v8, abi::stack_pointer(), sigbuf_off),
        abi::load_u64(&sc.v9, abi::stack_pointer(), siglen_off),
        abi::add_registers(&sc.v8, &sc.v8, &sc.v9), // %v8 = buffer end
        abi::add_immediate(&sc.v9, body, 2),
        abi::compare_registers(&sc.v9, &sc.v8),
        abi::branch_hi(bounds_fail),
        // tag byte must be 0x02
        abi::load_u8(&sc.v9, body, 0),
        abi::compare_immediate(&sc.v9, "2"),
        abi::branch_ne(fail),
        abi::load_u8(&sc.v10, body, 1),       // declared length
        abi::add_immediate(&sc.v11, body, 2), // int body ptr
        // advance `body` past this INTEGER now (2 + declared_len), before trimming.
        abi::add_immediate(body, body, 2),
        abi::add_registers(body, body, &sc.v10),
        // Reject an empty INTEGER and bound the declared content.
        abi::compare_immediate(&sc.v10, "0"),
        abi::branch_eq(bounds_fail),
        abi::load_u64(&sc.v8, abi::stack_pointer(), sigbuf_off),
        abi::load_u64(&sc.v9, abi::stack_pointer(), siglen_off),
        abi::add_registers(&sc.v8, &sc.v8, &sc.v9), // %v8 = buffer end
        abi::compare_registers(body, &sc.v8),
        abi::branch_hi(bounds_fail),
        // if int_body[0]==0 and len>1: skip the pad byte
        abi::load_u8(&sc.v9, &sc.v11, 0),
        abi::compare_immediate(&sc.v9, "0"),
        abi::branch_ne(&no_pad),
        abi::compare_immediate(&sc.v10, "1"),
        abi::branch_eq(&no_pad),
        abi::add_immediate(&sc.v11, &sc.v11, 1),
        abi::subtract_immediate(&sc.v10, &sc.v10, 1),
        abi::label(&no_pad),
        // len must be <= field
        abi::move_immediate(&sc.v12, "Integer", &field.to_string()),
        abi::compare_registers(&sc.v10, &sc.v12),
        abi::branch_hi(fail),
    ]);
    // zero the dst field, then copy len bytes to dst + (field - len).
    ins.extend([
        abi::move_immediate(&sc.v13, "Integer", "0"),
        abi::move_register(&sc.v14, dst),
        abi::label(&format!("{tag}_zl")),
        abi::compare_registers(&sc.v13, &sc.v12),
        abi::branch_eq(&format!("{tag}_zld")),
        abi::store_u8(abi::ZERO, &sc.v14, 0),
        abi::add_immediate(&sc.v14, &sc.v14, 1),
        abi::add_immediate(&sc.v13, &sc.v13, 1),
        abi::branch(&format!("{tag}_zl")),
        abi::label(&format!("{tag}_zld")),
        // dst_off = dst + (field - len)
        abi::subtract_registers(&sc.v12, &sc.v12, &sc.v10),
        abi::move_register(&sc.v14, dst),
        abi::add_registers(&sc.v14, &sc.v14, &sc.v12),
    ]);
    gen_cert::win_copy_bytes(sc, &sc.v11, &sc.v14, &sc.v10, tag, ins);
}

/// Open the ECDSA provider at `halg_off` and import the SEC1 public key at
/// `[key_ptr_off]` into `hkey_off` as an `ECCPUBLICBLOB`. Branches to `fail` if the
/// provider cannot be opened (a system error), and to `import_fail` if the blob fails to
/// import (an off-curve / otherwise-invalid key → `ErrInvalidArgument`).
#[allow(clippy::too_many_arguments)]
fn win_import_public_key(
    sc: &Sc,
    curve: VerifyCurve,
    symbol: &str,
    tag: &str,
    key_ptr_off: usize,
    blob_off: usize,
    halg_off: usize,
    hkey_off: usize,
    fail: &str,
    import_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let field = curve.field_len();
    let magic = curve.pub_magic();
    let body_len = 2 * field;
    let blob_len = 8 + 2 * field;
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
    // Build the blob: [magic(4)][cbKey=field(4)][X‖Y from SEC1+1].
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
    // BCryptImportKeyPair(hAlg, NULL, ECCPUBLICBLOB, &hKey, blob, blobLen, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    gen_cert::win_wide_addr(symbol, abi::c_arg(2), "ECCPUBLICBLOB", ins, rel);
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
    ins.push(abi::branch_lt(import_fail));
    Ok(())
}

/// Hash the message at `[msgbuf_off]`/`[msglen_off]` into `hashbuf_off` using the curve's
/// digest. Opens (and closes) its own hash provider at `hashalg_off`.
#[allow(clippy::too_many_arguments)]
fn win_hash_message(
    curve: VerifyCurve,
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
    let no_key = format!("{symbol}_vclean_nokey_{tag}");
    let no_alg = format!("{symbol}_vclean_noalg_{tag}");
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

/// Emit the Windows CNG ECDSA verify sequence for `curve`, self-contained. Imports the
/// SEC1 public key as an ECCPUBLICBLOB, hashes the message, DER-decodes the peer
/// signature into a fixed `r‖s`, and calls `BCryptVerifySignature`. Self-managed
/// fallible ABI; jumps `done`.
#[allow(clippy::too_many_arguments)]
fn emit_windows_verify(
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
    let field = curve.field_len();
    let pub_raw = 1 + 2 * field;
    const PUBCOLL: usize = 0;
    const MSGCOLL: usize = 8;
    const SIGCOLL: usize = 16;
    const PUBBUF: usize = 24;
    const PUBLEN: usize = 32;
    const MSGBUF: usize = 40;
    const MSGLEN: usize = 48;
    const SIGBUF: usize = 56;
    const SIGLEN: usize = 64;
    const HALG: usize = 72;
    const HKEY: usize = 80;
    const HASHALG: usize = 88;
    const BLOB: usize = 96;
    const RS: usize = 104;
    const HASHINLINE: usize = 112; // 64-byte inline hash scratch (112..176)
    let blobcap = 8 + 2 * 66;

    let fail = format!("{symbol}_{tag}_fail");
    let bad_sig = format!("{symbol}_{tag}_badsig");
    let invalid = format!("{symbol}_{tag}_invalid");
    let oob = format!("{symbol}_{tag}_oob");
    let alloc_fail = format!("{symbol}_{tag}_alloc_fail");

    let mut vregs = Vregs::new();
    let sc = Sc::new(&mut vregs);
    ins.extend([
        abi::store_u64(pub_op, abi::stack_pointer(), PUBCOLL),
        abi::store_u64(msg_op, abi::stack_pointer(), MSGCOLL),
        abi::store_u64(sig_op, abi::stack_pointer(), SIGCOLL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HKEY),
    ]);
    emit_read_byte_list(
        symbol,
        &format!("{tag}pub"),
        PUBCOLL,
        PUBBUF,
        PUBLEN,
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
    emit_read_byte_list(
        symbol,
        &format!("{tag}sig"),
        SIGCOLL,
        SIGBUF,
        SIGLEN,
        &alloc_fail,
        ins,
        rel,
    );
    // A public key that is not exactly one uncompressed SEC1 point is a malformed
    // argument, not a false verdict.
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), PUBLEN),
        abi::compare_immediate(&sc.v9, pub_raw.to_string()),
        abi::branch_ne(&invalid),
    ]);
    for (cap, slot) in [(blobcap, BLOB), (2 * 66, RS)] {
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

    win_import_public_key(
        &sc, curve, symbol, tag, PUBBUF, BLOB, HALG, HKEY, &fail, &invalid, imports, platform, ins,
        rel,
    )?;
    win_hash_message(
        curve, symbol, tag, MSGBUF, MSGLEN, HASHINLINE, HASHALG, &fail, imports, platform, ins, rel,
    )?;

    // DER-decode the untrusted signature into rs (r at +0, s at +field), zero-padded,
    // bounding every read against SIGLEN. Reading the SEQUENCE header needs >= 2 bytes.
    let seq_short = format!("{symbol}_{tag}_seqshort");
    let seq_body = format!("{symbol}_{tag}_seqbody");
    ins.extend([
        abi::load_u64(&sc.v9, abi::stack_pointer(), SIGLEN),
        abi::compare_immediate(&sc.v9, "2"),
        abi::branch_lt(&oob),
        abi::load_u64(&sc.v15, abi::stack_pointer(), SIGBUF),
        abi::load_u8(&sc.v9, &sc.v15, 0),
        abi::compare_immediate(&sc.v9, "48"), // SEQUENCE
        abi::branch_ne(&bad_sig),
        abi::load_u8(&sc.v9, &sc.v15, 1),
        abi::compare_immediate(&sc.v9, "128"),
        abi::branch_lt(&seq_short),
        // long form 0x81: body starts at +3
        abi::compare_immediate(&sc.v9, "129"),
        abi::branch_ne(&bad_sig),
        abi::add_immediate(&sc.v15, &sc.v15, 3),
        abi::branch(&seq_body),
        abi::label(&seq_short),
        abi::add_immediate(&sc.v15, &sc.v15, 2),
        abi::label(&seq_body),
        abi::load_u64(&sc.v6, abi::stack_pointer(), RS), // dst for r
    ]);
    win_der_decode_int(
        &sc,
        &sc.v15,
        &sc.v6,
        field,
        &format!("{symbol}_{tag}dr"),
        &bad_sig,
        SIGBUF,
        SIGLEN,
        &oob,
        ins,
    );
    ins.extend([
        abi::load_u64(&sc.v6, abi::stack_pointer(), RS),
        abi::add_immediate(&sc.v6, &sc.v6, field), // dst for s
    ]);
    win_der_decode_int(
        &sc,
        &sc.v15,
        &sc.v6,
        field,
        &format!("{symbol}_{tag}ds"),
        &bad_sig,
        SIGBUF,
        SIGLEN,
        &oob,
        ins,
    );

    // BCryptVerifySignature(hKey, NULL, hash, hashLen, rs, 2*field, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HASHINLINE),
        abi::move_immediate(abi::c_arg(3), "Integer", &curve.hash_len().to_string()),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), RS),
        abi::move_immediate(abi::c_arg(5), "Integer", &(2 * field).to_string()),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    gen_cert::bcrypt_call(
        symbol,
        "BCryptVerifySignature",
        7,
        imports,
        platform,
        ins,
        rel,
    )?;
    // bug-446: the NTSTATUS is a C result — it lands in the C-return register
    // (`c_return(0)` = `rax`), NOT `return_register()` (the aligned Win64 MFB result
    // bank = `rcx`, which still holds the stale first argument, the key handle). Read
    // the status from `c_return(0)` and sign-extend it out of the 32-bit `LONG` into a
    // callee-safe vreg that survives the cleanup calls (which clobber `rax`). The old
    // cng verify read `return_register()` = the non-zero handle → `!= 0` → always FALSE.
    ins.push(abi::sign_extend_word(&sc.v7, abi::c_return(0)));
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
    // status == 0 → valid; anything else (incl STATUS_INVALID_SIGNATURE) → FALSE.
    ins.extend([
        abi::compare_immediate(&sc.v7, "0"),
        abi::branch_ne(&bad_sig),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);

    // An out-of-bounds / short DER encoding is just an invalid signature (FALSE),
    // detected before any OOB read; fall through into bad_sig's cleanup+verdict.
    ins.push(abi::label(&oob));
    ins.push(abi::label(&bad_sig));
    win_cleanup(
        symbol,
        &format!("{tag}cb"),
        HKEY,
        HALG,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
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
    emit_fail(symbol, "ErrUnknown", ins, rel, done);
    // A malformed public key (wrong length, or a right-length off-curve key that
    // BCryptImportKeyPair rejects) is an argument error.
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
    emit_fail(symbol, "ErrInvalidArgument", ins, rel, done);
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
    emit_fail(symbol, "ErrOutOfMemory", ins, rel, done);
    Ok(())
}

/// The body's sp-relative scratch size — the max over the three platform layouts
/// (macOS reaches CONST_SCRATCH=208 → 216; Linux 136 → 144; Windows 112 + 64 = 176).
const LOCAL_SIZE: usize = 216;

/// The `AbiFunction` body for `crypto::verify`. `args[0]` is the `Certificate` ordinal,
/// `args[1]` the public-key collection pointer, `args[2]` the message collection
/// pointer, `args[3]` the signature collection pointer — each in its argument register.
/// Self-managed fallible ABI, so it returns the `void` sentinel.
pub(crate) fn lower_verify(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    let pub_op = args[1].location.clone();
    let msg_op = args[2].location.clone();
    let sig_op = args[3].location.clone();

    builder.allocate_stack_object("crypto_verify_scratch", LOCAL_SIZE);

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

    macro_rules! emit_curves {
        ($emit:ident, $p384:literal, $p521:literal, $t256:literal, $t384:literal, $t521:literal) => {{
            let p384 = format!("{symbol}_{}", $p384);
            let p521 = format!("{symbol}_{}", $p521);
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
            ]);
            // P-256 (ordinal 0) falls through here.
            $emit(
                VerifyCurve::P256,
                &symbol,
                $t256,
                pub_op.clone(),
                msg_op.clone(),
                sig_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            $emit(
                VerifyCurve::P384,
                &symbol,
                $t384,
                pub_op.clone(),
                msg_op.clone(),
                sig_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            $emit(
                VerifyCurve::P521,
                &symbol,
                $t521,
                pub_op.clone(),
                msg_op.clone(),
                sig_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }};
    }

    match platform.family() {
        PlatformFamily::MacOS => {
            emit_curves!(
                emit_macos_verify,
                "mp384",
                "mp521",
                "mp256",
                "mp384",
                "mp521"
            );
        }
        PlatformFamily::Linux => {
            emit_curves!(
                emit_linux_verify,
                "lp384",
                "lp521",
                "lp256",
                "lp384",
                "lp521"
            );
        }
        PlatformFamily::Windows => {
            emit_curves!(
                emit_windows_verify,
                "wp384",
                "wp521",
                "wp256",
                "wp384",
                "wp521"
            );
        }
    }

    // Ed25519 dispatch (all platforms): route the (publicKey, message, signature) triple
    // into the callee argument registers and call the software `__crypto_ed25519Verify`
    // MFB helper (always emitted with the crypto package). It leaves the Boolean verdict
    // in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed25519));
    builder.instructions.extend([
        abi::move_register(abi::argument_register(0)?, pub_op),
        abi::move_register(abi::argument_register(1)?, msg_op),
        abi::move_register(abi::argument_register(2)?, sig_op),
    ]);
    let ed_symbol = crate::target::shared::nir::function_symbol("#crypto_ed25519Verify");
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

    // X25519 keys cannot verify signatures.
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
        type_: "Boolean".to_string(),
        location: Operand::from("void"),
        text: "crypto.verify".to_string(),
    })
}

const INTRO: &str =
    r#"Verify a signature over a message with a public key of the given certificate type."#;
const DESC: &str = r#"`crypto::verify(type, publicKey, message, signature)` checks whether `signature`
is a valid signature over the raw bytes of `message` under `publicKey`, for the
NIST prime curve or `Ed25519` selected by `type` (a `crypto::Certificate`), and
returns a `Boolean`. `publicKey` is the `publicKey` field of the `crypto::KeyPair`
from `crypto::generate(type)` — for the NIST curves one SEC1 uncompressed point
`0x04‖X‖Y` (65/97/133 bytes for `P256`/`P384`/`P521`), for `Ed25519` the 32-byte
compressed point — and `signature` is the output of `crypto::sign(type, …)`.

It returns `TRUE` **if and only if** `signature` is a valid signature of that
exact `message` under that exact `publicKey`, and `FALSE` otherwise — a failed
verdict (wrong message, wrong key, or a corrupt/wrong-length `signature`) is a
normal outcome, not an error. For the NIST curves this is **FIPS 186-4 ECDSA**
with the curve's digest (SHA-256/384/512 for `P256`/`P384`/`P521`) and the
signature is an **ASN.1 DER** `Ecdsa-Sig-Value`. ECDSA signatures are **malleable**
and are not constrained here to a canonical low-S form, so a single message/key can
have more than one distinct `signature` that verifies `TRUE`; never use signature
bytes as a unique identifier (see `crypto::sign`). For `Ed25519` it is **RFC 8032
PureEdDSA** over the fixed 64-byte `R‖S`, which additionally rejects a
non-canonical scalar `S ≥ L` (returning `FALSE`) so a malleated `Ed25519` signature
cannot verify.

**Boundary and errors.** For the NIST curves a malformed `publicKey` — wrong
length, or a right-length off-curve point the platform import rejects — raises
`ErrInvalidArgument` (it is a caller mistake, not a false verdict). For `Ed25519`
a wrong-length key or signature is simply `FALSE`. `X25519` cannot verify and
raises `ErrInvalidArgument`. A platform-library or system failure raises
`ErrUnknown`, and an allocation failure raises `ErrOutOfMemory`. The untrusted
`signature` bytes are fully bounds-checked before use.

**Implementation.** ECDSA verification runs through the host platform key API,
reproduced clean-room in this member (no third-party crypto is bundled): on
**macOS** via Security.framework `SecKeyCreateWithData` + `SecKeyVerifySignature`
(`kSecKeyAlgorithmECDSASignatureMessageX962SHA256/384/512`); on **Linux** via
OpenSSL `libcrypto` `EVP_DigestVerifyInit` + one-shot `EVP_DigestVerify`
(`EVP_sha256/384/512`), the SEC1 point wrapped in a fixed SPKI DER prefix and
decoded with `d2i_PUBKEY`; on **Windows** via CNG `bcrypt.dll`
`BCryptImportKeyPair` (`ECCPUBLICBLOB`) + `BCryptHash` + `BCryptVerifySignature`,
the DER signature decoded here to the fixed `r‖s`. `Ed25519` is a pure in-process
MFBASIC software core (RFC 8032) with **no platform library**, byte-identical on
every OS. A signature or key produced on one OS verifies on the others."#;
const EX: &str = r#"```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generate(Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::verify(Certificate.P256, kp.publicKey, msg, sig)
  io::print(toString(ok))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "verify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The certificate/key type of the public key.",
                    aliases: &[],
                    ty: ParameterType::Named("Certificate"),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "publicKey",
                    desc: "The public key bytes (the `publicKey` field of a `crypto::KeyPair`).",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "message",
                    desc: "The message bytes that were signed.",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "signature",
                    desc: "The signature bytes to verify.",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrInvalidArgument", "ErrOutOfMemory", "ErrUnknown"],
            body: Body::abi_function(lower_verify),
        }],
    });
}
