//! `crypto::generate(type)` — a clean-room `AbiFunction` key-pair generator.
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`),
//! this member's `Body::abi_function` body branches on the enum ordinal and, for
//! the three NIST-EC curves, generates the key pair **from scratch** here — it does
//! not call into `crypto/native/*` (the migrated per-curve `generateP*`). It is
//! *modeled on* those (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG) but reproduces
//! the platform sequence in this file, self-contained, driving the general
//! marshallers for the `List OF Byte`/`KeyPair` output. `Ed25519` dispatches to
//! [`super::helper_generate_ed25519`].
//!
//! Status: macOS (SecKey) is implemented; Linux/Windows raise a codegen error until
//! their clean-room sequences land (per-platform-verified rollout).

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

// ---------------------------------------------------------------------------
// macOS SecKey / CoreFoundation clean-room seam. Own symbol names (distinct from
// `crypto/native/macos`'s `_mfb_crypto_ec_*`) so the two data-object sets never
// collide when both are in a plan.
// ---------------------------------------------------------------------------
const MACSEC: &str = "/System/Library/Frameworks/Security.framework/Security";
const MACCF: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
const SECPATH_SYMBOL: &str = "_mfb_crypto_generate_secpath";
const CFPATH_SYMBOL: &str = "_mfb_crypto_generate_cfpath";
const RTLD_NOW: &str = "2";
const CF_NUMBER_INT_TYPE: &str = "9"; // kCFNumberIntType

const SYMBOLS: &[&str] = &[
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
];

fn sym(name: &str) -> String {
    format!("_mfb_crypto_generate_sym_{name}")
}

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

// ---------------------------------------------------------------------------
// Linux OpenSSL (libcrypto) clean-room seam. Own symbol names (distinct from
// `crypto/native/openssl`'s `_mfb_crypto_ec_*`).
// ---------------------------------------------------------------------------
const LIBCRYPTO3: &str = "libcrypto.so.3";
const LIBCRYPTO11: &str = "libcrypto.so.1.1";
const EVP_PKEY_EC: &str = "408";
const LIB3_SYMBOL: &str = "_mfb_crypto_generate_lib3";
const LIB11_SYMBOL: &str = "_mfb_crypto_generate_lib11";

const OSSL_SYMBOLS: &[&str] = &[
    "i2d_PrivateKey",
    "i2d_PUBKEY",
    "EVP_PKEY_free",
    "EVP_PKEY_new",
    "EVP_PKEY_assign",
    "EVP_EC_gen",
    "EC_KEY_new_by_curve_name",
    "EC_KEY_generate_key",
    "EC_KEY_free",
];

// Per-curve OpenSSL parameters (ordinal-indexed): (curve-name, nid, point_len,
// field_len, sec1_scalar_off, spki_prefix_len).
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

fn ossl_sym(name: &str) -> String {
    format!("_mfb_crypto_generate_ossl_{name}")
}

fn ossl_name_sym(name: &str) -> String {
    format!("_mfb_crypto_generate_ossl_name_{name}")
}

// ---------------------------------------------------------------------------
// Windows CNG/BCrypt clean-room seam. The `BCrypt*` entry points link through
// the import table (Win64 ABI); only the wide (UTF-16LE) `LPCWSTR` algorithm and
// blob-type identifiers need data objects. Own symbol names (distinct from
// `crypto/native/cng`'s `_mfb_crypto_ec_w_*`).
// ---------------------------------------------------------------------------
const BLOBCAP: usize = 8 + 3 * 66; // header + X‖Y‖d for the widest curve (P-521)

const WIN_WIDE_IDS: &[&str] = &["ECDSA_P256", "ECDSA_P384", "ECDSA_P521", "ECCPRIVATEBLOB"];

// Per-curve CNG parameters (ordinal-indexed): (algo-id, field_len). The raw
// (`1+3·field`) and public (`1+2·field`) lengths are derived at runtime.
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

fn win_sym(name: &str) -> String {
    format!("_mfb_crypto_generate_w_{name}")
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

/// Read-only C strings this member references, for the target family. Emitted by
/// the driver gate when `crypto.generate` is in the plan.
pub(crate) fn data_objects(family: PlatformFamily) -> Vec<CodeDataObject> {
    match family {
        PlatformFamily::MacOS => {
            let mut objects = vec![
                raw_cstr(SECPATH_SYMBOL, MACSEC),
                raw_cstr(CFPATH_SYMBOL, MACCF),
            ];
            for name in SYMBOLS {
                objects.push(raw_cstr(&sym(name), name));
            }
            objects
        }
        PlatformFamily::Linux => {
            let mut objects = vec![
                raw_cstr(LIB3_SYMBOL, LIBCRYPTO3),
                raw_cstr(LIB11_SYMBOL, LIBCRYPTO11),
            ];
            for name in OSSL_SYMBOLS {
                objects.push(raw_cstr(&ossl_sym(name), name));
            }
            for c in OSSL_CURVES {
                objects.push(raw_cstr(&ossl_name_sym(c.name), c.name));
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
// Reproduced crypto-private emit helpers (clean-room; NOT calls into
// `crypto/native/*`). They emit into raw instruction/relocation vecs so the body
// can drive them with `&mut builder.instructions` / `&mut builder.relocations`.
// ---------------------------------------------------------------------------

/// `dlopen(path, RTLD_NOW)` into `handle_off`; branch to `fail` if NULL.
#[allow(clippy::too_many_arguments)]
fn dlopen_one(
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
fn dlsym_into(
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
    emit_data_address(symbol, abi::c_arg(1), &sym(name), ins, rel);
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
fn load_cf_const(
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

/// Call the function pointer stored at `fn_off` (args already staged). `scratch` is
/// a minted vreg the pointer is loaded into.
fn call_fn(fn_off: usize, scratch: &str, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64(scratch, abi::stack_pointer(), fn_off),
        abi::branch_link_register(scratch),
    ]);
}

/// `CFRelease(*obj_off)` using the CFRelease pointer at `release_off`.
fn cf_release(release_off: usize, obj_off: usize, scratch: &str, ins: &mut Vec<CodeInstruction>) {
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        obj_off,
    ));
    call_fn(release_off, scratch, ins);
}

/// `CFRelease(*obj_off)` only when the slot is non-NULL (error-exit cleanup; slots
/// are zero-initialised at entry).
fn cf_release_guarded(
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

    dlopen_one(
        symbol,
        SECPATH_SYMBOL,
        SEC,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    dlopen_one(
        symbol,
        CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    dlsym_into(
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
    dlsym_into(
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
        abi::move_immediate(abi::c_arg(1), "Integer", CF_NUMBER_INT_TYPE),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), NUMVAL),
    ]);
    call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), NUM),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);

    // Attributes dict { kSecAttrKeyType: EC, kSecAttrKeySizeInBits: <number> }.
    load_cf_const(
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
    load_cf_const(
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
    load_cf_const(
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
    dlsym_into(
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
    dlsym_into(
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
    dlsym_into(
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
    call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DICT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // key = SecKeyCreateRandomKey(dict, NULL)
    dlsym_into(
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
    call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // data = SecKeyCopyExternalRepresentation(key, NULL) -> 0x04||X||Y||K
    dlsym_into(
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
    call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DATA),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // raw = CFDataGetBytePtr(data); len = CFDataGetLength(data); private = List OF Byte.
    dlsym_into(
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
    call_fn(FN, v9, &mut builder.instructions);
    builder.instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        BYTEPTR,
    ));
    dlsym_into(
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
    call_fn(FN, v9, &mut builder.instructions);
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
    cf_release(RELEASE, NUM, v9, &mut builder.instructions);
    cf_release(RELEASE, DICT, v9, &mut builder.instructions);
    cf_release(RELEASE, KEY, v9, &mut builder.instructions);
    cf_release(RELEASE, DATA, v9, &mut builder.instructions);

    let scratch = RecordBuildScratch {
        size: RSIZE,
        result: RRESULT,
        cursor: RCURSOR,
        block_size: RBLOCK,
    };
    emit_build_inlined_record(
        symbol,
        "kp",
        "KeyPair",
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
        cf_release_guarded(symbol, RELEASE, NUM, &format!("{tag}n"), v9, ins);
        cf_release_guarded(symbol, RELEASE, DICT, &format!("{tag}d"), v9, ins);
        cf_release_guarded(symbol, RELEASE, KEY, &format!("{tag}k"), v9, ins);
        cf_release_guarded(symbol, RELEASE, DATA, &format!("{tag}a"), v9, ins);
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

// ---------------------------------------------------------------------------
// Linux OpenSSL clean-room emit helpers (reproduced; not calls into
// `crypto/native/*`).
// ---------------------------------------------------------------------------

/// `dlopen(libcrypto.so.3, RTLD_NOW)` with a `.so.1.1` fallback, into `handle_off`.
fn dlopen_libcrypto(
    symbol: &str,
    handle_off: usize,
    fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let loaded = format!("{symbol}_libc_loaded");
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
fn ossl_dlsym_into(
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
    emit_data_address(symbol, abi::c_arg(1), &ossl_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// `dlsym` into `dst_off`, branching to `absent` if NULL (optional `EVP_EC_gen`).
#[allow(clippy::too_many_arguments)]
fn ossl_dlsym_probe(
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
    emit_data_address(symbol, abi::c_arg(1), &ossl_sym(name), ins, rel);
    platform.emit_external_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(absent),
    ]);
    Ok(())
}

/// Copy `[len_off]` bytes from `[src_ptr_off] + [src_add_off]` to
/// `[dst_ptr_off] + [dst_add_off]` (all runtime). Call-free (vreg scratch).
#[allow(clippy::too_many_arguments)]
fn emit_copy_runtime(
    symbol: &str,
    tag: &str,
    src_ptr_off: usize,
    src_add_off: Option<usize>,
    dst_ptr_off: usize,
    dst_add_off: Option<usize>,
    len_off: usize,
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
    if let Some(off) = src_add_off {
        ins.extend([
            abi::load_u64(v12, abi::stack_pointer(), off),
            abi::add_registers(v9, v9, v12),
        ]);
    }
    ins.push(abi::load_u64(v10, abi::stack_pointer(), dst_ptr_off));
    if let Some(off) = dst_add_off {
        ins.extend([
            abi::load_u64(v12, abi::stack_pointer(), off),
            abi::add_registers(v10, v10, v12),
        ]);
    }
    ins.extend([
        abi::move_immediate(v11, "Integer", "0"),
        abi::load_u64(v13, abi::stack_pointer(), len_off),
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

    dlopen_libcrypto(
        symbol,
        L_HANDLE,
        &load_fail,
        imports,
        platform,
        ins,
        &mut builder.relocations,
    )?;
    // Resolve the free functions up front so the error cleanup can always run.
    ossl_dlsym_into(
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
    ossl_dlsym_into(
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
    ossl_dlsym_probe(
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::branch(&have_pkey),
    ]);

    // OpenSSL 1.1: EC_KEY_new_by_curve_name(nid) + generate + EVP_PKEY_assign.
    ins.push(abi::label(&eckey_path));
    ossl_dlsym_into(
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), L_ECKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);
    ossl_dlsym_into(
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
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&gen_fail),
    ]);
    ossl_dlsym_into(
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), L_PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);
    ossl_dlsym_into(
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
        abi::move_immediate(abi::c_arg(1), "Integer", EVP_PKEY_EC),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), L_ECKEY),
        abi::load_u64(v9, abi::stack_pointer(), L_FN),
        abi::branch_link_register(v9),
        abi::compare_immediate(abi::return_register(), "1"),
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
    ossl_dlsym_into(
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), L_SEC1LEN),
        abi::compare_immediate(abi::return_register(), "0"),
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
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&gen_fail),
    ]);

    // SPKI = i2d_PUBKEY(pkey)
    ossl_dlsym_into(
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), L_SPKILEN),
        abi::compare_immediate(abi::return_register(), "0"),
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
        abi::compare_immediate(abi::return_register(), "0"),
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
    emit_copy_runtime(
        symbol,
        "pt",
        L_SPKIPTR,
        Some(L_SPKIPREFIX),
        L_RAWBUF,
        None,
        L_POINTLEN,
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
    emit_copy_runtime(
        symbol,
        "sc",
        L_SEC1PTR,
        Some(L_SEC1OFF),
        L_RAWBUF,
        Some(L_POINTLEN),
        L_FIELDLEN,
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
        "KeyPair",
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

// ---------------------------------------------------------------------------
// Windows CNG clean-room emit helpers (reproduced; not calls into
// `crypto/native/*`).
// ---------------------------------------------------------------------------

/// Emit a Win64 external `BCrypt*` call: args 0..=3 preloaded in
/// `return_register`/`ARG[1..3]`, args 4.. spilled to the stack tail above the
/// shadow space. Sign-extends the NTSTATUS return (`< 0` fails).
fn bcrypt_call(
    from: &str,
    name: &str,
    n_args: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Win64 requires the caller to reserve ≥32 bytes of shadow (home) space below
    // the outgoing stack args for EVERY call — even a ≤4-arg one — or the callee
    // clobbers the caller's `[sp..sp+0x20]` locals when it homes its register args.
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
    bcrypt_call(symbol, "BCryptDestroyKey", 1, imports, platform, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_HKEY),
        abi::label(&no_key),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HALG),
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
    bcrypt_call(
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
    bcrypt_call(
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
    bcrypt_call(
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
        abi::move_immediate(abi::return_register(), "Integer", &BLOBCAP.to_string()),
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

    // BCryptExportKey(hKey, NULL, L"ECCPRIVATEBLOB", blob, BLOBCAP, &cbResult, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_HKEY),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    emit_data_address(
        symbol,
        abi::c_arg(2),
        &win_sym("ECCPRIVATEBLOB"),
        ins,
        &mut builder.relocations,
    );
    ins.extend([
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), W_BLOB),
        abi::move_immediate(abi::c_arg(4), "Integer", &BLOBCAP.to_string()),
        abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), W_CBRES),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    bcrypt_call(
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
    emit_zero_guarded(symbol, W_BLOB, None, BLOBCAP, "wblobz", ins);

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
        "KeyPair",
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
                abi::compare_immediate(&ord, ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, ORD_ED25519),
                abi::branch_eq(&ed25519),
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
                emit_data_address(&sym_owned, v9, &ossl_name_sym(c.name), ins, rel);
                ins.extend([
                    abi::store_u64(v9, abi::stack_pointer(), L_NAMEPTR),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, ORD_P384),
                abi::branch_eq(&lp384),
                abi::compare_immediate(&ord, ORD_P521),
                abi::branch_eq(&lp521),
                abi::compare_immediate(&ord, ORD_ED25519),
                abi::branch_eq(&ed25519),
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
                emit_data_address(&sym_owned, v9, &win_sym(c.algo), ins, rel);
                ins.extend([
                    abi::store_u64(v9, abi::stack_pointer(), W_ALGOPTR),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, ORD_P384),
                abi::branch_eq(&wp384),
                abi::compare_immediate(&ord, ORD_P521),
                abi::branch_eq(&wp521),
                abi::compare_immediate(&ord, ORD_ED25519),
                abi::branch_eq(&ed25519),
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

    // Ed25519 dispatch (all platforms). TODO: route to the software
    // `__crypto_generateEd25519` helper; a trappable error for now.
    builder.instructions.push(abi::label(&ed25519));
    emit_fail(
        &symbol,
        "ErrUnknown",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        type_: "KeyPair".to_string(),
        location: Operand::from("void"),
        text: "crypto.generate".to_string(),
    })
}

const INTRO: &str = r#"Generate a fresh key pair of the requested certificate type."#;
const DESC: &str = r#"`crypto::generate(type)` creates a new key pair for the NIST-EC curve or
Ed25519 selected by `type` (a `crypto::Certificate`), returning a
`crypto::KeyPair`. The EC pairs (`P256`/`P384`/`P521`) are usable with the
matching `p*Sign`/`p*Verify`; `Ed25519` with `ed25519Sign`/`ed25519Verify`."#;
const EX: &str = r#"```
IMPORT crypto

SUB main()
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.P256)
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
                ty: ParameterType::Named("Certificate"),
                default: crate::codegen::registry::DefaultValue::None,
            }],
            return_type: ParameterType::Named("KeyPair"),
            errors: vec![],
            body: Body::abi_function(lower_generate),
        }],
    });
}
