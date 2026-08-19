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
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Certificate ordinals (must match the `Certificate` enum declaration order in
// `crypto/mod.rs`).
// ---------------------------------------------------------------------------
// P-256 is the fall-through ordinal (never compared), but named for the contract.
#[allow(dead_code)]
const ORD_P256: &str = "0";
const ORD_P384: &str = "1";
const ORD_P521: &str = "2";
const ORD_ED25519: &str = "3";

const RTLD_NOW: &str = "2";

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
    fn tmpl_hex(self) -> &'static str {
        match self {
            SignCurve::P256 => P256_PKCS8_TMPL,
            SignCurve::P384 => P384_PKCS8_TMPL,
            SignCurve::P521 => P521_PKCS8_TMPL,
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

// ---------------------------------------------------------------------------
// macOS SecKey / CoreFoundation clean-room seam. Own symbol names (distinct from
// `crypto/native/macos`'s `_mfb_crypto_ec_*` and `func_generate`'s
// `_mfb_crypto_generate_*`) so the data-object sets never collide in one plan.
// ---------------------------------------------------------------------------
const MACSEC: &str = "/System/Library/Frameworks/Security.framework/Security";
const MACCF: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
const MAC_SECPATH_SYMBOL: &str = "_mfb_crypto_sign_secpath";
const MAC_CFPATH_SYMBOL: &str = "_mfb_crypto_sign_cfpath";

const MAC_SYMBOLS: &[&str] = &[
    "CFDataCreate",
    "CFDataGetBytePtr",
    "CFDataGetLength",
    "CFRelease",
    "CFDictionaryCreate",
    "SecKeyCreateWithData",
    "SecKeyCreateSignature",
    "kSecAttrKeyType",
    "kSecAttrKeyTypeECSECPrimeRandom",
    "kSecAttrKeyClass",
    "kSecAttrKeyClassPrivate",
    "kCFTypeDictionaryKeyCallBacks",
    "kCFTypeDictionaryValueCallBacks",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
];

fn mac_sym(name: &str) -> String {
    format!("_mfb_crypto_sign_sym_{name}")
}

// ---------------------------------------------------------------------------
// Linux OpenSSL (libcrypto) clean-room seam. Own symbol names.
// ---------------------------------------------------------------------------
const LIBCRYPTO3: &str = "libcrypto.so.3";
const LIBCRYPTO11: &str = "libcrypto.so.1.1";
const LX_LIB3_SYMBOL: &str = "_mfb_crypto_sign_lib3";
const LX_LIB11_SYMBOL: &str = "_mfb_crypto_sign_lib11";

const OSSL_SYMBOLS: &[&str] = &[
    "d2i_AutoPrivateKey",
    "EVP_PKEY_free",
    "EVP_MD_CTX_new",
    "EVP_MD_CTX_free",
    "EVP_sha256",
    "EVP_sha384",
    "EVP_sha512",
    "EVP_DigestSignInit",
    "EVP_DigestSign",
];

const P256_PKCS8_TMPL: &str = "308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b02010104200000000000000000000000000000000000000000000000000000000000000000a1440342000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const P384_PKCS8_TMPL: &str = "3081b6020100301006072a8648ce3d020106052b8104002204819e30819b0201010430000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a16403620000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const P521_PKCS8_TMPL: &str = "3081ee020100301006072a8648ce3d020106052b810400230481d63081d30201010442000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a181890381860000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

fn lx_sym(name: &str) -> String {
    format!("_mfb_crypto_sign_ossl_{name}")
}

fn lx_tmpl_sym(name: &str) -> String {
    format!("_mfb_crypto_sign_tmpl_{name}")
}

// ---------------------------------------------------------------------------
// Windows CNG/BCrypt clean-room seam. The `BCrypt*` entry points link through the
// import table (Win64 ABI); only the wide (UTF-16LE) `LPCWSTR` algorithm/hash/blob
// identifiers need data objects. Own symbol names.
// ---------------------------------------------------------------------------
const WIN_WIDE_IDS: &[&str] = &[
    "ECDSA_P256",
    "ECDSA_P384",
    "ECDSA_P521",
    "SHA256",
    "SHA384",
    "SHA512",
    "ECCPRIVATEBLOB",
];

fn win_sym(name: &str) -> String {
    format!("_mfb_crypto_sign_w_{name}")
}

// ---------------------------------------------------------------------------
// Data-object constructors (clean-room; distinct symbol names).
// ---------------------------------------------------------------------------
fn raw_cstr(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: hex_encode_cstring(text),
    }
}

fn raw_data(symbol: &str, hex: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "raw bytes".to_string(),
        align: 1,
        size: hex.len() / 2,
        value: hex.to_string(),
    }
}

/// UTF-16LE, NUL-terminated hex for a CNG `LPCWSTR` (ASCII input only).
fn utf16z_hex(text: &str) -> String {
    let mut hex = String::new();
    for ch in text.chars() {
        let cp = ch as u32;
        hex.push_str(&format!("{:02x}{:02x}", cp & 0xff, (cp >> 8) & 0xff));
    }
    hex.push_str("0000");
    hex
}

fn wide_cstr(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "UTF-16LE string (NUL-terminated)".to_string(),
        align: 2,
        size: (text.len() + 1) * 2,
        value: utf16z_hex(text),
    }
}

/// Read-only data objects this member references, for the target family. Emitted
/// by the driver gate when `crypto.sign` is in the plan.
pub(crate) fn data_objects(family: PlatformFamily) -> Vec<CodeDataObject> {
    match family {
        PlatformFamily::MacOS => {
            let mut objects = vec![
                raw_cstr(MAC_SECPATH_SYMBOL, MACSEC),
                raw_cstr(MAC_CFPATH_SYMBOL, MACCF),
            ];
            for name in MAC_SYMBOLS {
                objects.push(raw_cstr(&mac_sym(name), name));
            }
            objects
        }
        PlatformFamily::Linux => {
            let mut objects = vec![
                raw_cstr(LX_LIB3_SYMBOL, LIBCRYPTO3),
                raw_cstr(LX_LIB11_SYMBOL, LIBCRYPTO11),
            ];
            for name in OSSL_SYMBOLS {
                objects.push(raw_cstr(&lx_sym(name), name));
            }
            for c in [SignCurve::P256, SignCurve::P384, SignCurve::P521] {
                objects.push(raw_data(&lx_tmpl_sym(c.name()), c.tmpl_hex()));
            }
            objects
        }
        PlatformFamily::Windows => WIN_WIDE_IDS
            .iter()
            .map(|id| wide_cstr(&win_sym(id), id))
            .collect(),
    }
}

/// Call the function pointer stored at `fn_off` (args already staged). `scratch` is
/// a minted vreg the pointer is loaded into. Shared by macOS/Linux seams.
fn call_fn(fn_off: usize, scratch: &str, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64(scratch, abi::stack_pointer(), fn_off),
        abi::branch_link_register(scratch),
    ]);
}

// ===========================================================================
// macOS SecKey clean-room emit helpers (reproduced; not calls into native).
// ===========================================================================

#[allow(clippy::too_many_arguments)]
fn mac_dlopen_one(
    symbol: &str,
    path_symbol: &str,
    handle_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_data_address(symbol, abi::return_register(), path_symbol, ins, rel);
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
    ]);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn mac_dlsym_into(
    symbol: &str,
    handle_off: usize,
    name: &str,
    dst_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        handle_off,
    ));
    emit_data_address(symbol, abi::c_arg(1), &mac_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn mac_load_cf_const(
    symbol: &str,
    handle_off: usize,
    name: &str,
    dst_off: usize,
    scratch_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    mac_dlsym_into(
        symbol,
        handle_off,
        name,
        scratch_off,
        fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(scratch, abi::stack_pointer(), scratch_off),
        abi::load_u64(scratch, scratch, 0),
        abi::store_u64(scratch, abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// Build a 2-entry CFDictionary of CFString constants into `dict_off`. Uses six
/// contiguous scratch slots at `scratch_off` (keys[0,8], vals[16,24],
/// callbacks[32,40]) plus `const_scratch` for the per-constant address.
#[allow(clippy::too_many_arguments)]
fn mac_build_dict2(
    symbol: &str,
    sec_off: usize,
    cf_off: usize,
    fn_off: usize,
    k0: &str,
    k1: &str,
    v0: &str,
    v1: &str,
    scratch_off: usize,
    const_scratch: usize,
    dict_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    mac_load_cf_const(
        symbol,
        sec_off,
        k0,
        scratch_off,
        const_scratch,
        fail,
        imports,
        platform,
        scratch,
        ins,
        rel,
    )?;
    mac_load_cf_const(
        symbol,
        sec_off,
        k1,
        scratch_off + 8,
        const_scratch,
        fail,
        imports,
        platform,
        scratch,
        ins,
        rel,
    )?;
    mac_load_cf_const(
        symbol,
        sec_off,
        v0,
        scratch_off + 16,
        const_scratch,
        fail,
        imports,
        platform,
        scratch,
        ins,
        rel,
    )?;
    mac_load_cf_const(
        symbol,
        sec_off,
        v1,
        scratch_off + 24,
        const_scratch,
        fail,
        imports,
        platform,
        scratch,
        ins,
        rel,
    )?;
    mac_dlsym_into(
        symbol,
        cf_off,
        "kCFTypeDictionaryKeyCallBacks",
        scratch_off + 32,
        fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    mac_dlsym_into(
        symbol,
        cf_off,
        "kCFTypeDictionaryValueCallBacks",
        scratch_off + 40,
        fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    mac_dlsym_into(
        symbol,
        cf_off,
        "CFDictionaryCreate",
        fn_off,
        fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), scratch_off),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), scratch_off + 16),
        abi::move_immediate(abi::c_arg(3), "Integer", "2"),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), scratch_off + 32),
        abi::load_u64(abi::c_arg(5), abi::stack_pointer(), scratch_off + 40),
    ]);
    call_fn(fn_off, scratch, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        dict_off,
    ));
    Ok(())
}

/// `CFRelease(*obj_off)` using the CFRelease pointer at `release_off`.
fn mac_cf_release(
    release_off: usize,
    obj_off: usize,
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        obj_off,
    ));
    call_fn(release_off, scratch, ins);
}

/// `CFRelease(*obj_off)` only when the slot is non-NULL (error-exit cleanup).
fn mac_cf_release_guarded(
    symbol: &str,
    release_off: usize,
    obj_off: usize,
    tag: &str,
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let skip = format!("{symbol}_{tag}_norel");
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), obj_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip),
    ]);
    call_fn(release_off, scratch, ins);
    ins.push(abi::label(&skip));
}

/// `dst = CFDataCreate(NULL, *buf_off, *len_off)` (CFDataCreate pointer at fn_off).
fn mac_cfdata_create(
    fn_off: usize,
    buf_off: usize,
    len_off: usize,
    dst_off: usize,
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), buf_off),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), len_off),
    ]);
    call_fn(fn_off, scratch, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        dst_off,
    ));
}

/// Extract the bytes of the CFData at `data_off` into a fresh `List OF Byte` at
/// `coll_off` (via CFDataGetBytePtr/CFDataGetLength).
#[allow(clippy::too_many_arguments)]
fn mac_cfdata_to_list(
    symbol: &str,
    tag: &str,
    cf_off: usize,
    data_off: usize,
    fn_off: usize,
    byteptr_off: usize,
    bytelen_off: usize,
    coll_off: usize,
    load_fail: &str,
    alloc_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    scratch: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    mac_dlsym_into(
        symbol,
        cf_off,
        "CFDataGetBytePtr",
        fn_off,
        load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        data_off,
    ));
    call_fn(fn_off, scratch, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        byteptr_off,
    ));
    mac_dlsym_into(
        symbol,
        cf_off,
        "CFDataGetLength",
        fn_off,
        load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        data_off,
    ));
    call_fn(fn_off, scratch, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        bytelen_off,
    ));
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_{tag}_out_build_loop"),
        &format!("{symbol}_{tag}_out_build_done"),
        byteptr_off,
        bytelen_off,
        Some(coll_off),
        abi::mfb_return(1),
        alloc_fail,
        ins,
        rel,
    );
    Ok(())
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

    mac_dlopen_one(
        symbol,
        MAC_SECPATH_SYMBOL,
        SEC,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    mac_dlopen_one(
        symbol,
        MAC_CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        ins,
        rel,
    )?;
    mac_dlsym_into(
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
    mac_dlsym_into(
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
    mac_cfdata_create(FN, PRIVBUF, PRIVLEN, PRIVDATA, &v9, ins);
    mac_cfdata_create(FN, MSGBUF, MSGLEN, MSGDATA, &v9, ins);

    mac_build_dict2(
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
    mac_dlsym_into(
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
    call_fn(FN, &v9, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    mac_load_cf_const(
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
    mac_dlsym_into(
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
    call_fn(FN, &v9, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SIGDATA),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&sign_fail),
    ]);

    mac_cfdata_to_list(
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

    mac_cf_release(RELEASE, PRIVDATA, &v9, ins);
    mac_cf_release(RELEASE, MSGDATA, &v9, ins);
    mac_cf_release(RELEASE, DICT, &v9, ins);
    mac_cf_release(RELEASE, KEY, &v9, ins);
    mac_cf_release(RELEASE, SIGDATA, &v9, ins);
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
        mac_cf_release_guarded(symbol, RELEASE, PRIVDATA, &format!("{ctag}p"), &v9, ins);
        mac_cf_release_guarded(symbol, RELEASE, MSGDATA, &format!("{ctag}m"), &v9, ins);
        mac_cf_release_guarded(symbol, RELEASE, DICT, &format!("{ctag}d"), &v9, ins);
        mac_cf_release_guarded(symbol, RELEASE, KEY, &format!("{ctag}k"), &v9, ins);
        mac_cf_release_guarded(symbol, RELEASE, SIGDATA, &format!("{ctag}s"), &v9, ins);
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

fn lx_dlopen_libcrypto(
    symbol: &str,
    tag: &str,
    handle_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let loaded = format!("{symbol}_{tag}_libc_loaded");
    emit_data_address(symbol, abi::return_register(), LX_LIB3_SYMBOL, ins, rel);
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&loaded),
    ]);
    emit_data_address(symbol, abi::return_register(), LX_LIB11_SYMBOL, ins, rel);
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::label(&loaded),
    ]);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lx_dlsym_into(
    symbol: &str,
    handle_off: usize,
    name: &str,
    dst_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        handle_off,
    ));
    emit_data_address(symbol, abi::c_arg(1), &lx_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// Copy `n` bytes from `[src_ptr_off] + src_const + [src_runtime_off]` to
/// `[dst_ptr_off] + dst_const`. Call-free (vreg scratch only).
#[allow(clippy::too_many_arguments)]
fn lx_copy(
    symbol: &str,
    tag: &str,
    src_ptr_off: usize,
    src_const: usize,
    src_runtime_off: Option<usize>,
    dst_ptr_off: usize,
    dst_const: usize,
    n: usize,
    v9: &str,
    v10: &str,
    v11: &str,
    v12: &str,
    v13: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let loop_l = format!("{symbol}_{tag}_cpy");
    let done_l = format!("{symbol}_{tag}_cpyend");
    ins.push(abi::load_u64(v9, abi::stack_pointer(), src_ptr_off));
    if src_const > 0 {
        ins.push(abi::add_immediate(v9, v9, src_const));
    }
    if let Some(off) = src_runtime_off {
        ins.extend([
            abi::load_u64(v12, abi::stack_pointer(), off),
            abi::add_registers(v9, v9, v12),
        ]);
    }
    ins.push(abi::load_u64(v10, abi::stack_pointer(), dst_ptr_off));
    if dst_const > 0 {
        ins.push(abi::add_immediate(v10, v10, dst_const));
    }
    ins.extend([
        abi::move_immediate(v11, "Integer", "0"),
        abi::move_immediate(v13, "Integer", &n.to_string()),
        abi::label(&loop_l),
        abi::compare_registers(v11, v13),
        abi::branch_eq(&done_l),
        abi::load_u8(v12, v9, 0),
        abi::store_u8(v12, v10, 0),
        abi::add_immediate(v9, v9, 1),
        abi::add_immediate(v10, v10, 1),
        abi::add_immediate(v11, v11, 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}

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
    lx_dlsym_into(
        symbol, handle_off, free_name, fn_off, raw_fail, imports, platform, ins, rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        obj_off,
    ));
    call_fn(fn_off, scratch, ins);
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

    lx_dlopen_libcrypto(symbol, tag, HANDLE, &load_fail, imports, platform, ins, rel)?;

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
    emit_data_address(symbol, &v9, &lx_tmpl_sym(curve.name()), ins, rel);
    ins.push(abi::store_u64(&v9, abi::stack_pointer(), TMPLPTR));
    lx_copy(
        symbol,
        &format!("{tag}tmpl"),
        TMPLPTR,
        0,
        None,
        DERBUF,
        0,
        pkcs8_len,
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );
    // raw key = 0x04||X||Y||K = point(point_len) || scalar(field_len)
    lx_copy(
        symbol,
        &format!("{tag}pt"),
        PRIVBUF,
        0,
        None,
        DERBUF,
        p8_point_off,
        point_len,
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );
    lx_copy(
        symbol,
        &format!("{tag}sc"),
        PRIVBUF,
        point_len,
        None,
        DERBUF,
        p8_scalar_off,
        field_len,
        &v9,
        &v10,
        &v11,
        &v12,
        &v13,
        ins,
    );

    // pkey = d2i_AutoPrivateKey(NULL, &pp, len)
    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MDCTX,
    ));
    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
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
    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&sign_fail),
    ]);

    // siglen probe: EVP_DigestSign(ctx, NULL, &siglen, msg, msglen)
    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
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
    call_fn(FN, &v9, ins);
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

    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
    lx_dlsym_into(
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
    call_fn(FN, &v9, ins);
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

/// Emit a Win64 external `BCrypt*` call with the mandatory ≥32-byte shadow-space
/// reservation for EVERY call (even a ≤4-arg one). Sign-extends the NTSTATUS.
fn bcrypt_call(
    from: &str,
    name: &str,
    n_args: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let stack = n_args.saturating_sub(4);
    let frame = (0x20 + stack * 8 + 15) & !15;
    ins.push(abi::subtract_stack(frame));
    for i in 0..stack {
        ins.push(abi::store_u64(
            abi::c_arg(4 + i),
            abi::stack_pointer(),
            0x20 + i * 8,
        ));
    }
    platform.emit_external_call(name, from, imports, ins, rel)?;
    ins.push(abi::add_stack(frame));
    ins.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
    Ok(())
}

/// Minted scratch-vreg palette for one emitted CNG sequence (threaded into every
/// emitter contributing to it so all uses of one hand-picked number map to one vreg).
struct Sc {
    v4: String,
    v5: String,
    v7: String,
    v9: String,
    v10: String,
    v11: String,
    v12: String,
    v13: String,
    v14: String,
    v15: String,
}

impl Sc {
    fn new(vregs: &mut Vregs) -> Self {
        // Order preserved from the CNG reference (v4..v15) so the copy scratch
        // (v4/v5) stays distinct from the caller-used registers.
        let v4 = vregs.next();
        let v5 = vregs.next();
        let _v6 = vregs.next();
        let v7 = vregs.next();
        let _v8 = vregs.next();
        let v9 = vregs.next();
        let v10 = vregs.next();
        let v11 = vregs.next();
        let v12 = vregs.next();
        let v13 = vregs.next();
        let v14 = vregs.next();
        let v15 = vregs.next();
        Sc {
            v4,
            v5,
            v7,
            v9,
            v10,
            v11,
            v12,
            v13,
            v14,
            v15,
        }
    }
}

fn win_wide_addr(
    from: &str,
    dst: impl Into<Operand>,
    id: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    emit_data_address(from, dst, &win_sym(id), ins, rel);
}

/// A copy loop: `count` bytes from `[src]` to `[dst]` (both register operands,
/// consumed). Uses `sc.v4`/`sc.v5` scratch named by `tag`.
fn win_copy_bytes(
    sc: &Sc,
    src: &str,
    dst: &str,
    count: &str,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let loop_l = format!("{tag}_cp");
    let done_l = format!("{tag}_cpd");
    ins.extend([
        abi::move_immediate(&sc.v4, "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers(&sc.v4, count),
        abi::branch_eq(&done_l),
        abi::load_u8(&sc.v5, src, 0),
        abi::store_u8(&sc.v5, dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::add_immediate(&sc.v4, &sc.v4, 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}

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
    win_copy_bytes(sc, &sc.v9, dst, &sc.v13, tag, ins);
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
    win_wide_addr(symbol, abi::c_arg(1), curve.algo_id(), ins, rel);
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    bcrypt_call(
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
    win_copy_bytes(
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
    win_wide_addr(symbol, abi::c_arg(2), "ECCPRIVATEBLOB", ins, rel);
    ins.extend([
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), hkey_off),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), blob_off),
        abi::move_immediate(abi::c_arg(5), "Integer", &blob_len.to_string()),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    bcrypt_call(
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
    win_wide_addr(symbol, abi::c_arg(1), curve.hash_id(), ins, rel);
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    bcrypt_call(
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
    bcrypt_call(symbol, "BCryptHash", 7, imports, platform, ins, rel)?;
    ins.push(abi::branch_lt(&hash_fail));
    // Close the hash provider (success path).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hashalg_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    bcrypt_call(
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
    bcrypt_call(
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
    bcrypt_call(symbol, "BCryptDestroyKey", 1, imports, platform, ins, rel)?;
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), hkey_off));
    ins.push(abi::label(&no_key));
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_alg),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    bcrypt_call(
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
    const BLOBCAP: usize = 8 + 3 * 66;

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
    for (cap, slot) in [(BLOBCAP, BLOB), (2 * 66, RS), (16 + 4 * 66, DERBUF)] {
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
    bcrypt_call(symbol, "BCryptSignHash", 8, imports, platform, ins, rel)?;
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
    emit_zero_guarded(symbol, BLOB, None, BLOBCAP, &format!("{tag}blobz"), ins);
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
    let imports = ctx.platform_imports;
    let platform = ctx.platform;

    match platform.family() {
        PlatformFamily::MacOS => {
            let p384 = format!("{symbol}_mp384");
            let p521 = format!("{symbol}_mp521");
            builder.instructions.extend([
                abi::compare_immediate(&ord, ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, ORD_ED25519),
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
                abi::compare_immediate(&ord, ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, ORD_ED25519),
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
                abi::compare_immediate(&ord, ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, ORD_ED25519),
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

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        type_: "List OF Byte".to_string(),
        location: Operand::from("void"),
        text: "crypto.sign".to_string(),
    })
}

const INTRO: &str = r#"Sign a message with a private key of the given certificate type."#;
const DESC: &str = r#"`crypto::sign(type, privateKey, message)` produces a signature over `message`
using `privateKey`, for the NIST-EC curve or Ed25519 selected by `type` (a
`crypto::Certificate`). The private key is the `privateKey` field of the
`crypto::KeyPair` returned by `crypto::generate(type)`.

For the EC curves (`P256`/`P384`/`P521`) the result is an ASN.1 DER
`Ecdsa-Sig-Value` signature, verifiable with the matching `p*Verify`. For
`Ed25519` it is the fixed 64-byte raw signature (`R‖S`), verifiable with
`ed25519Verify`. The output is returned as a `List OF Byte`."#;
const EX: &str = r#"```
IMPORT crypto
IMPORT strings

SUB main()
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(crypto::Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::p256Verify(kp.publicKey, msg, sig)
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
                    ty: ParameterType::Named("Certificate"),
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
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_sign),
        }],
    });
}
