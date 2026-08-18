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
    emit_build_byte_list, emit_build_inlined_record, RecordBuildScratch,
};
use crate::codegen::registry::AbiCtx;
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Certificate ordinals (must match the `Certificate` enum declaration order in
// `crypto/mod.rs`).
// ---------------------------------------------------------------------------
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

/// Read-only C strings (framework paths + dlsym names) this member references.
/// Emitted by the driver gate when `crypto.generate` is in the plan.
pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    let mut objects = vec![
        raw_cstr(SECPATH_SYMBOL, MACSEC),
        raw_cstr(CFPATH_SYMBOL, MACCF),
    ];
    for name in SYMBOLS {
        objects.push(raw_cstr(&sym(name), name));
    }
    objects
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
            return Err(
                "crypto::generate: clean-room Linux (EVP_PKEY) keygen not yet implemented".into(),
            );
        }
        PlatformFamily::Windows => {
            return Err(
                "crypto::generate: clean-room Windows (CNG) keygen not yet implemented".into(),
            );
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
