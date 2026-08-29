//! Shared clean-room codegen seam for the `Certificate`-typed `AbiFunction` crypto
//! members (`crypto::generate`, `crypto::sign`, and `crypto::verify`).
//!
//! These members reproduce the platform key sequences (macOS `SecKey`/CoreFoundation,
//! Linux OpenSSL `libcrypto`, Windows CNG/BCrypt) *from scratch* rather than calling
//! into `crypto/native/*`. The low-level seam — dlopen/dlsym marshalling, CoreFoundation
//! object plumbing, the OpenSSL byte-copy loops, and the Win64 `BCrypt*` call shim —
//! is identical across those members, so it lives here once.
//!
//! Data objects (framework paths, dlsym names, PKCS#8 templates, CNG wide ids) are
//! owned here too, under a single `_mfb_crypto_cert_*` symbol space (the union of every
//! name generate/sign/verify reference). The driver gate in `engine/builder/mod.rs`
//! emits [`data_objects`] once when any of those members is in the plan.

use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::emit_build_byte_list;
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Certificate ordinals (must match the `Certificate` enum declaration order in
// `crypto/mod.rs`).
// ---------------------------------------------------------------------------
// P-256 is the fall-through ordinal (never compared), but named for the contract.
#[allow(dead_code)]
pub(crate) const ORD_P256: &str = "0";
pub(crate) const ORD_P384: &str = "1";
pub(crate) const ORD_P521: &str = "2";
pub(crate) const ORD_ED25519: &str = "3";
pub(crate) const ORD_X25519: &str = "4";
pub(crate) const ORD_X448: &str = "5";

const RTLD_NOW: &str = "2";

// ---------------------------------------------------------------------------
// macOS SecKey / CoreFoundation clean-room seam. One unified `_mfb_crypto_cert_*`
// symbol space (distinct from `crypto/native/macos`'s `_mfb_crypto_ec_*`).
// ---------------------------------------------------------------------------
const MACSEC: &str = "/System/Library/Frameworks/Security.framework/Security";
const MACCF: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
pub(crate) const SECPATH_SYMBOL: &str = "_mfb_crypto_cert_secpath";
pub(crate) const CFPATH_SYMBOL: &str = "_mfb_crypto_cert_cfpath";
pub(crate) const CF_NUMBER_INT_TYPE: &str = "9"; // kCFNumberIntType

/// Union of every macOS dlsym name `crypto::generate`/`crypto::sign`/`crypto::verify`
/// reference.
const MAC_SYMBOLS: &[&str] = &[
    "CFNumberCreate",
    "CFDictionaryCreate",
    "CFRelease",
    "SecKeyCreateRandomKey",
    "SecKeyCopyExternalRepresentation",
    "CFDataGetBytePtr",
    "CFDataGetLength",
    "kSecAttrKeyType",
    "kSecAttrKeySizeInBits",
    "kSecAttrKeyTypeECSECPrimeRandom",
    "kCFTypeDictionaryKeyCallBacks",
    "kCFTypeDictionaryValueCallBacks",
    "CFDataCreate",
    "SecKeyCreateWithData",
    "SecKeyCreateSignature",
    "SecKeyVerifySignature",
    "kSecAttrKeyClass",
    "kSecAttrKeyClassPrivate",
    "kSecAttrKeyClassPublic",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
];

fn cert_sym(name: &str) -> String {
    format!("_mfb_crypto_cert_sym_{name}")
}

// ---------------------------------------------------------------------------
// Linux OpenSSL (libcrypto) clean-room seam.
// ---------------------------------------------------------------------------
const LIBCRYPTO3: &str = "libcrypto.so.3";
const LIBCRYPTO11: &str = "libcrypto.so.1.1";
pub(crate) const EVP_PKEY_EC: &str = "408";
const LIB3_SYMBOL: &str = "_mfb_crypto_cert_lib3";
const LIB11_SYMBOL: &str = "_mfb_crypto_cert_lib11";

/// Union of every OpenSSL dlsym name generate/sign/verify reference.
const OSSL_SYMBOLS: &[&str] = &[
    "i2d_PrivateKey",
    "i2d_PUBKEY",
    "d2i_PUBKEY",
    "EVP_PKEY_free",
    "EVP_PKEY_new",
    "EVP_PKEY_assign",
    "EVP_EC_gen",
    "EC_KEY_new_by_curve_name",
    "EC_KEY_generate_key",
    "EC_KEY_free",
    "d2i_AutoPrivateKey",
    "EVP_MD_CTX_new",
    "EVP_MD_CTX_free",
    "EVP_sha256",
    "EVP_sha384",
    "EVP_sha512",
    "EVP_DigestSignInit",
    "EVP_DigestSign",
    "EVP_DigestVerifyInit",
    "EVP_DigestVerify",
];

/// The NIST curve names (used for both the OpenSSL curve-name C strings that
/// `EVP_EC_gen` consumes and the PKCS#8-template symbol suffixes).
const CURVE_NAMES: &[&str] = &["P-256", "P-384", "P-521"];

/// OpenSSL PKCS#8 templates (private key with zeroed point/scalar to splice), indexed
/// by [`CURVE_NAMES`]. Referenced by `crypto::sign` (and, next, `crypto::verify`).
const PKCS8_TEMPLATES: &[&str] = &[
    "308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b02010104200000000000000000000000000000000000000000000000000000000000000000a1440342000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "3081b6020100301006072a8648ce3d020106052b8104002204819e30819b0201010430000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a16403620000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "3081ee020100301006072a8648ce3d020106052b810400230481d63081d30201010442000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a181890381860000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
];

/// OpenSSL SubjectPublicKeyInfo (SPKI) DER prefixes, indexed by [`CURVE_NAMES`]. The
/// public key is `prefix || SEC1-point`, decoded via `d2i_PUBKEY` by `crypto::verify`.
const SPKI_PREFIXES: &[&str] = &[
    "3059301306072a8648ce3d020106082a8648ce3d030107034200",
    "3076301006072a8648ce3d020106052b81040022036200",
    "30819b301006072a8648ce3d020106052b8104002303818600",
];

fn cert_ossl_sym(name: &str) -> String {
    format!("_mfb_crypto_cert_ossl_{name}")
}

/// The OpenSSL curve-name C string (`EVP_EC_gen(name)` / `EC_KEY_new_by_curve_name`).
pub(crate) fn ossl_name_sym(name: &str) -> String {
    format!("_mfb_crypto_cert_ossl_name_{name}")
}

/// The spliceable PKCS#8 DER template for `name` (a [`CURVE_NAMES`] entry).
pub(crate) fn tmpl_sym(name: &str) -> String {
    format!("_mfb_crypto_cert_tmpl_{name}")
}

/// The SPKI DER prefix for `name` (a [`CURVE_NAMES`] entry); prepended to the raw
/// SEC1 point to form the DER public key `crypto::verify` feeds to `d2i_PUBKEY`.
pub(crate) fn spki_sym(name: &str) -> String {
    format!("_mfb_crypto_cert_spki_{name}")
}

/// The byte length of the SPKI DER prefix for `name` (a [`CURVE_NAMES`] entry).
pub(crate) fn spki_prefix_len(name: &str) -> usize {
    let i = CURVE_NAMES.iter().position(|c| *c == name).expect("curve");
    SPKI_PREFIXES[i].len() / 2
}

// ---------------------------------------------------------------------------
// Windows CNG/BCrypt clean-room seam. The `BCrypt*` entry points link through the
// import table (Win64 ABI); only the wide (UTF-16LE) `LPCWSTR` algorithm/hash/blob
// identifiers need data objects.
// ---------------------------------------------------------------------------
pub(crate) const BLOBCAP: usize = 8 + 3 * 66; // header + X‖Y‖d for the widest curve (P-521)

/// Union of every CNG wide id generate/sign/verify reference.
const WIN_WIDE_IDS: &[&str] = &[
    "ECDSA_P256",
    "ECDSA_P384",
    "ECDSA_P521",
    "ECCPRIVATEBLOB",
    "ECCPUBLICBLOB",
    "SHA256",
    "SHA384",
    "SHA512",
];

/// The wide (`LPCWSTR`) CNG identifier symbol for `name`.
pub(crate) fn win_sym(name: &str) -> String {
    format!("_mfb_crypto_cert_w_{name}")
}

// ---------------------------------------------------------------------------
// Data-object constructors (clean-room; unified symbol space).
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

/// Read-only data objects the `Certificate` `AbiFunction` members reference, for the
/// target family. Emitted once by the driver gate when any of them is in the plan.
pub(crate) fn data_objects(family: PlatformFamily) -> Vec<CodeDataObject> {
    match family {
        PlatformFamily::MacOS => {
            let mut objects = vec![
                raw_cstr(SECPATH_SYMBOL, MACSEC),
                raw_cstr(CFPATH_SYMBOL, MACCF),
            ];
            for name in MAC_SYMBOLS {
                objects.push(raw_cstr(&cert_sym(name), name));
            }
            objects
        }
        PlatformFamily::Linux => {
            let mut objects = vec![
                raw_cstr(LIB3_SYMBOL, LIBCRYPTO3),
                raw_cstr(LIB11_SYMBOL, LIBCRYPTO11),
            ];
            for name in OSSL_SYMBOLS {
                objects.push(raw_cstr(&cert_ossl_sym(name), name));
            }
            for name in CURVE_NAMES {
                objects.push(raw_cstr(&ossl_name_sym(name), name));
            }
            for (i, name) in CURVE_NAMES.iter().enumerate() {
                objects.push(raw_data(&tmpl_sym(name), PKCS8_TEMPLATES[i]));
            }
            for (i, name) in CURVE_NAMES.iter().enumerate() {
                objects.push(raw_data(&spki_sym(name), SPKI_PREFIXES[i]));
            }
            objects
        }
        PlatformFamily::Windows => WIN_WIDE_IDS
            .iter()
            .map(|id| wide_cstr(&win_sym(id), id))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Shared low-level emit helpers (clean-room; NOT calls into `crypto/native/*`).
// They emit into raw instruction/relocation vecs so a member body can drive them
// with `&mut builder.instructions` / `&mut builder.relocations`.
// ---------------------------------------------------------------------------

/// Call the function pointer stored at `fn_off` (args already staged). `scratch` is
/// a minted vreg the pointer is loaded into.
pub(crate) fn call_fn(fn_off: usize, scratch: &str, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64(scratch, abi::stack_pointer(), fn_off),
        abi::branch_link_register(scratch),
    ]);
}

// ===========================================================================
// macOS SecKey / CoreFoundation seam.
// ===========================================================================

/// `dlopen(path, RTLD_NOW)` into `handle_off`; branch to `fail` if NULL.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dlopen_one(
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

/// `dlsym(handle, name)` into `dst_off`; branch to `fail` if NULL.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dlsym_into(
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
    emit_data_address(symbol, abi::c_arg(1), &cert_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// Resolve a CFString constant (dlsym returns its address; dereference once) and
/// store the CFStringRef value into `dst_off`. `scratch` holds the address.
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_cf_const(
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
    dlsym_into(
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
pub(crate) fn build_dict2(
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
    load_cf_const(
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
    load_cf_const(
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
    load_cf_const(
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
    load_cf_const(
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
    dlsym_into(
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
    dlsym_into(
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
    dlsym_into(
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
pub(crate) fn cf_release(
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

/// `CFRelease(*obj_off)` only when the slot is non-NULL (error-exit cleanup; slots
/// are zero-initialised at entry).
pub(crate) fn cf_release_guarded(
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
pub(crate) fn cfdata_create(
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
pub(crate) fn cfdata_to_list(
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
    dlsym_into(
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
    dlsym_into(
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

// ===========================================================================
// Linux OpenSSL (libcrypto) seam.
// ===========================================================================

/// `dlopen(libcrypto.so.3, RTLD_NOW)` with a `.so.1.1` fallback, into `handle_off`.
/// `tag` disambiguates the per-sequence "loaded" label when a member emits more than
/// one libcrypto load.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dlopen_libcrypto(
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
    emit_data_address(symbol, abi::return_register(), LIB3_SYMBOL, ins, rel);
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&loaded),
    ]);
    emit_data_address(symbol, abi::return_register(), LIB11_SYMBOL, ins, rel);
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

/// `dlsym(handle, name)` into `dst_off`; branch to `fail` if NULL.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ossl_dlsym_into(
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
    emit_data_address(symbol, abi::c_arg(1), &cert_ossl_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// `dlsym` into `dst_off`, branching to `absent` if NULL (optional symbol probe, e.g.
/// OpenSSL 3.x `EVP_EC_gen`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ossl_dlsym_probe(
    symbol: &str,
    handle_off: usize,
    name: &str,
    dst_off: usize,
    absent: &str,
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
    emit_data_address(symbol, abi::c_arg(1), &cert_ossl_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(absent),
    ]);
    Ok(())
}

/// The length of a [`copy_bytes`] run: either a compile-time constant, or read at
/// runtime from a stack slot.
pub(crate) enum CopyLen {
    Const(usize),
    Runtime(usize),
}

/// Byte-copy loop: copy `len` bytes from `[src_ptr_off] + src_const + [src_runtime_off]`
/// to `[dst_ptr_off] + dst_const + [dst_runtime_off]` (the `_const` addends compile-time,
/// the `_runtime_off` addends read from stack slots). Call-free (vreg scratch only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn copy_bytes(
    symbol: &str,
    tag: &str,
    src_ptr_off: usize,
    src_const: usize,
    src_runtime_off: Option<usize>,
    dst_ptr_off: usize,
    dst_const: usize,
    dst_runtime_off: Option<usize>,
    len: CopyLen,
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
    if let Some(off) = dst_runtime_off {
        ins.extend([
            abi::load_u64(v12, abi::stack_pointer(), off),
            abi::add_registers(v10, v10, v12),
        ]);
    }
    ins.push(abi::move_immediate(v11, "Integer", "0"));
    match len {
        CopyLen::Const(n) => ins.push(abi::move_immediate(v13, "Integer", &n.to_string())),
        CopyLen::Runtime(off) => ins.push(abi::load_u64(v13, abi::stack_pointer(), off)),
    }
    ins.extend([
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

// ===========================================================================
// Windows CNG/BCrypt seam.
// ===========================================================================

/// Emit a Win64 external `BCrypt*` call: args 0..=3 preloaded in
/// `return_register`/`ARG[1..3]`, args 4.. spilled to the stack tail above the shadow
/// space. Sign-extends the NTSTATUS return (`< 0` fails).
pub(crate) fn bcrypt_call(
    from: &str,
    name: &str,
    n_args: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Win64 requires the caller to reserve ≥32 bytes of shadow (home) space below the
    // outgoing stack args for EVERY call — even a ≤4-arg one — or the callee clobbers
    // the caller's `[sp..sp+0x20]` locals when it homes its register args.
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
pub(crate) struct Sc {
    pub(crate) v4: String,
    pub(crate) v5: String,
    pub(crate) v6: String,
    pub(crate) v7: String,
    pub(crate) v8: String,
    pub(crate) v9: String,
    pub(crate) v10: String,
    pub(crate) v11: String,
    pub(crate) v12: String,
    pub(crate) v13: String,
    pub(crate) v14: String,
    pub(crate) v15: String,
}

impl Sc {
    pub(crate) fn new(vregs: &mut Vregs) -> Self {
        // Order preserved from the CNG reference (v4..v15) so the copy scratch
        // (v4/v5) stays distinct from the caller-used registers. `v6`/`v8` are the
        // `crypto::verify` DER-decode scratch (an RS destination pointer and a
        // buffer-end bound); `crypto::sign` simply never reads them.
        let v4 = vregs.next();
        let v5 = vregs.next();
        let v6 = vregs.next();
        let v7 = vregs.next();
        let v8 = vregs.next();
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
            v6,
            v7,
            v8,
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

/// Load the address of the wide (`LPCWSTR`) CNG identifier `id` into `dst`.
pub(crate) fn win_wide_addr(
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
pub(crate) fn win_copy_bytes(
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
