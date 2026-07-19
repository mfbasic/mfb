//! macOS `SecKey` (Security.framework) + CoreFoundation backend for the
//! `crypto::` NIST-EC helpers. Everything is a synchronous dlopen/dlsym bridge
//! (no dispatch blocks, unlike the TLS backend). See the parent module for the
//! wire-compatible key/signature encodings.

use std::collections::HashMap;

use super::super::*;
use super::{emit_build_byte_list, emit_fail, emit_read_byte_list, Curve, EcOp};
use crate::target::shared::abi;

const MACSEC: &str = "/System/Library/Frameworks/Security.framework/Security";
const MACCF: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
const SECPATH_SYMBOL: &str = "_mfb_crypto_ec_secpath";
const CFPATH_SYMBOL: &str = "_mfb_crypto_ec_cfpath";
const RTLD_NOW: &str = "2";
const CF_NUMBER_INT_TYPE: &str = "9"; // kCFNumberIntType

/// Function and CFString/callback constant names resolved via dlsym.
const SYMBOLS: &[&str] = &[
    "CFDataCreate",
    "CFDataGetBytePtr",
    "CFDataGetLength",
    "CFNumberCreate",
    "CFDictionaryCreate",
    "CFRelease",
    "SecKeyCreateRandomKey",
    "SecKeyCopyExternalRepresentation",
    "SecKeyCreateWithData",
    "SecKeyCreateSignature",
    "SecKeyVerifySignature",
    "kSecAttrKeyType",
    "kSecAttrKeySizeInBits",
    "kSecAttrKeyTypeECSECPrimeRandom",
    "kSecAttrKeyClass",
    "kSecAttrKeyClassPrivate",
    "kSecAttrKeyClassPublic",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
    "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
    "kCFTypeDictionaryKeyCallBacks",
    "kCFTypeDictionaryValueCallBacks",
];

fn sym(name: &str) -> String {
    format!("_mfb_crypto_ec_sym_{name}")
}

fn raw_cstr(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: super::super::tls::hex_encode_cstring(text),
    }
}

/// Read-only C strings (framework paths + dlsym names) referenced by the macOS
/// EC helpers. Emitted once when any EC helper is in the plan.
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

/// Load the address of a read-only data symbol into `dst` (adrp + add).
fn data_address(
    from: &str,
    dst: &str,
    data_symbol: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(
        CodeInstruction::new("adrp")
            .field("dst", dst)
            .field("symbol", data_symbol),
    );
    ins.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", dst)
            .field("src", dst)
            .field("symbol", data_symbol),
    );
    rel.extend([
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

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
    data_address(symbol, abi::return_register(), path_symbol, ins, rel);
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", RTLD_NOW));
    platform.emit_libc_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
    ]);
    Ok(())
}

/// `dlsym(handle, name)` into `dst_off`; branch to `fail` if NULL. The stored
/// value is a function pointer or the *address* of a data constant.
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
    data_address(symbol, abi::ARG[1], &sym(name), ins, rel);
    platform.emit_libc_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// Resolve a CFString constant (dlsym returns its address; dereference once) and
/// store the CFStringRef value into `dst_off`. `scratch_off` holds the address.
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
        abi::load_u64("%v9", abi::stack_pointer(), scratch_off),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// Call the function pointer stored at `fn_off` (args already in x0..). Result
/// left in the return register.
fn call_fn(fn_off: usize, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), fn_off),
        abi::branch_link_register("%v9"),
    ]);
}

/// `CFRelease(*obj_off)` using the CFRelease pointer at `release_off`.
fn cf_release(release_off: usize, obj_off: usize, ins: &mut Vec<CodeInstruction>) {
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        obj_off,
    ));
    call_fn(release_off, ins);
}

/// `CFRelease(*obj_off)` only when the slot is non-NULL. Used on the error exits
/// where a CF object may or may not have been created yet; the slots are
/// zero-initialised at entry so a NULL slot is skipped rather than passed to
/// `CFRelease` (which crashes on NULL). `tag` disambiguates the skip label per
/// call site. The `CFRelease` function pointer is only dereferenced when the
/// object is non-NULL, which implies it was resolved before the object was
/// created — so an error before `CFRelease` is `dlsym`d cannot use a garbage
/// pointer.
fn cf_release_guarded(
    symbol: &str,
    release_off: usize,
    obj_off: usize,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let skip = format!("{symbol}_{tag}_norel");
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), obj_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip),
    ]);
    call_fn(release_off, ins);
    ins.push(abi::label(&skip));
}

/// Overwrite `[*buf_off]`.. (`[len_off]` bytes) with zero when the buffer slot
/// is non-NULL. Wipes raw key-material scratch (e.g. the private scalar copied
/// out of an argument byte list) before the helper returns, so a later
/// same-program arena allocation cannot be handed a block still holding key
/// bytes. Call-free (vreg scratch only); `tag` disambiguates the labels.
fn zero_scratch_guarded(
    symbol: &str,
    buf_off: usize,
    len_off: usize,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let skip = format!("{symbol}_{tag}_noz");
    let loop_l = format!("{symbol}_{tag}_zl");
    let end_l = format!("{symbol}_{tag}_ze");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), buf_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::move_immediate("%v11", "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers("%v11", "%v10"),
        abi::branch_eq(&end_l),
        abi::store_u8(abi::ZERO, "%v9", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v11", "%v11", 1),
        abi::branch(&loop_l),
        abi::label(&end_l),
        abi::label(&skip),
    ]);
}

/// Build a 2-entry CFDictionary of CFString constants into `dict_off`. Uses six
/// contiguous scratch slots at `scratch_off` (keys[0,8], vals[16,24],
/// callbacks[32,40]) plus `const_scratch` for the per-constant address.
#[allow(clippy::too_many_arguments)]
fn build_dict2(
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
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), scratch_off),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), scratch_off + 16),
        abi::move_immediate(abi::ARG[3], "Integer", "2"),
        abi::load_u64(abi::ARG[4], abi::stack_pointer(), scratch_off + 32),
        abi::load_u64(abi::ARG[5], abi::stack_pointer(), scratch_off + 40),
    ]);
    call_fn(fn_off, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        dict_off,
    ));
    Ok(())
}

pub(super) fn lower(
    op: EcOp,
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match op {
        EcOp::Generate => generate(curve, symbol, imports, platform),
        EcOp::Sign => sign(curve, symbol, imports, platform),
        EcOp::Verify => verify(curve, symbol, imports, platform),
    }
}

fn generate(
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const SEC: usize = 0;
    const CF: usize = 8;
    const FN: usize = 16;
    const RELEASE: usize = 24;
    const NUMVAL: usize = 32;
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
    const COLL: usize = 136;
    const SCRATCH: usize = 144;
    const LOCAL_SIZE: usize = 160;

    let load_fail = format!("{symbol}_load_fail");
    let gen_fail = format!("{symbol}_gen_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();

    // Zero the CF object slots so the error-exit cleanup can null-guard each
    // CFRelease (the frame is not zero-initialised) — bug-55.
    ins.extend([
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
        &mut ins,
        &mut rel,
    )?;
    dlopen_one(
        symbol,
        CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym_into(
        symbol,
        CF,
        "CFRelease",
        RELEASE,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    // CFNumber for the key size.
    ins.extend([
        abi::move_immediate("%v9", "Integer", curve.bits()),
        abi::store_u64("%v9", abi::stack_pointer(), NUMVAL),
    ]);
    dlsym_into(
        symbol,
        CF,
        "CFNumberCreate",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", CF_NUMBER_INT_TYPE),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), NUMVAL),
    ]);
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        NUM,
    ));
    // bug-237: CFNumberCreate returns NULL under memory pressure. The attributes
    // dict is built with kCFTypeDictionaryValueCallBacks, whose retain callback
    // would run CFRetain(NULL) on a NULL value. Bail to the error exit instead.
    ins.extend([
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
        &mut ins,
        &mut rel,
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
        &mut ins,
        &mut rel,
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
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), NUM),
        abi::store_u64("%v9", abi::stack_pointer(), VALS + 8),
    ]);
    dlsym_into(
        symbol,
        CF,
        "kCFTypeDictionaryKeyCallBacks",
        KEYCB,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym_into(
        symbol,
        CF,
        "kCFTypeDictionaryValueCallBacks",
        VALCB,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym_into(
        symbol,
        CF,
        "CFDictionaryCreate",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), KEYS),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), VALS),
        abi::move_immediate(abi::ARG[3], "Integer", "2"),
        abi::load_u64(abi::ARG[4], abi::stack_pointer(), KEYCB),
        abi::load_u64(abi::ARG[5], abi::stack_pointer(), VALCB),
    ]);
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        DICT,
    ));

    // key = SecKeyCreateRandomKey(dict, NULL)
    dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateRandomKey",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DICT),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // data = SecKeyCopyExternalRepresentation(key, NULL)  -> 0x04||X||Y||K
    dlsym_into(
        symbol,
        SEC,
        "SecKeyCopyExternalRepresentation",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DATA),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    emit_cfdata_to_list(
        symbol,
        CF,
        DATA,
        FN,
        BYTEPTR,
        BYTELEN,
        COLL,
        &load_fail,
        &alloc_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    cf_release(RELEASE, NUM, &mut ins);
    cf_release(RELEASE, DICT, &mut ins);
    cf_release(RELEASE, KEY, &mut ins);
    cf_release(RELEASE, DATA, &mut ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Every error exit releases exactly the CF objects (NUM, DICT, KEY, DATA)
    // the success path releases; the slots are zero-initialised so a release is
    // a no-op until the object exists, and CFRelease is only dereferenced when
    // an object is non-NULL (bug-55).
    let cleanup = |ins: &mut Vec<CodeInstruction>, tag: &str| {
        cf_release_guarded(symbol, RELEASE, NUM, &format!("{tag}n"), ins);
        cf_release_guarded(symbol, RELEASE, DICT, &format!("{tag}d"), ins);
        cf_release_guarded(symbol, RELEASE, KEY, &format!("{tag}k"), ins);
        cf_release_guarded(symbol, RELEASE, DATA, &format!("{tag}a"), ins);
    };
    ins.push(abi::label(&load_fail));
    cleanup(&mut ins, "lf");
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&gen_fail));
    cleanup(&mut ins, "gf");
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    cleanup(&mut ins, "af");
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}

fn sign(
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
    const LOCAL_SIZE: usize = 208;

    let load_fail = format!("{symbol}_load_fail");
    let invalid_fail = format!("{symbol}_invalid_fail");
    let sign_fail = format!("{symbol}_sign_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();

    // Stash the two collection arguments before anything clobbers x0/x1.
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PRIVCOLL),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MSGCOLL),
    ]);
    // Zero the CF object slots and the private-scalar scratch pointer so the
    // error-exit cleanup can null-guard each CFRelease / wipe (bug-55).
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
        "priv",
        PRIVCOLL,
        PRIVBUF,
        PRIVLEN,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    emit_read_byte_list(
        symbol,
        "msg",
        MSGCOLL,
        MSGBUF,
        MSGLEN,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );

    dlopen_one(
        symbol,
        SECPATH_SYMBOL,
        SEC,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlopen_one(
        symbol,
        CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym_into(
        symbol,
        CF,
        "CFRelease",
        RELEASE,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    // privData = CFDataCreate(NULL, privBuf, privLen); msgData = CFDataCreate(...)
    dlsym_into(
        symbol,
        CF,
        "CFDataCreate",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_cfdata_create(FN, PRIVBUF, PRIVLEN, PRIVDATA, &mut ins);
    emit_cfdata_create(FN, MSGBUF, MSGLEN, MSGDATA, &mut ins);

    build_dict2(
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
        &mut ins,
        &mut rel,
    )?;

    // key = SecKeyCreateWithData(privData, dict, NULL)
    dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateWithData",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PRIVDATA),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DICT),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    load_cf_const(
        symbol,
        SEC,
        curve.macos_algorithm(),
        ALGO,
        CONST_SCRATCH,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    // sigData = SecKeyCreateSignature(key, algo, msgData, NULL)
    dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateSignature",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), ALGO),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MSGDATA),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SIGDATA),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&sign_fail),
    ]);

    emit_cfdata_to_list(
        symbol,
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
        &mut ins,
        &mut rel,
    )?;

    cf_release(RELEASE, PRIVDATA, &mut ins);
    cf_release(RELEASE, MSGDATA, &mut ins);
    cf_release(RELEASE, DICT, &mut ins);
    cf_release(RELEASE, KEY, &mut ins);
    cf_release(RELEASE, SIGDATA, &mut ins);
    // Wipe the private-scalar scratch copied out of the argument byte list.
    zero_scratch_guarded(symbol, PRIVBUF, PRIVLEN, "privS", &mut ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Every error exit releases the CF objects (PRIVDATA/MSGDATA/DICT/KEY/
    // SIGDATA) the success path releases and wipes the private scratch; slots
    // are zero-initialised so releases/wipes are no-ops until they exist
    // (bug-55).
    let cleanup = |ins: &mut Vec<CodeInstruction>, tag: &str| {
        cf_release_guarded(symbol, RELEASE, PRIVDATA, &format!("{tag}p"), ins);
        cf_release_guarded(symbol, RELEASE, MSGDATA, &format!("{tag}m"), ins);
        cf_release_guarded(symbol, RELEASE, DICT, &format!("{tag}d"), ins);
        cf_release_guarded(symbol, RELEASE, KEY, &format!("{tag}k"), ins);
        cf_release_guarded(symbol, RELEASE, SIGDATA, &format!("{tag}s"), ins);
        zero_scratch_guarded(symbol, PRIVBUF, PRIVLEN, &format!("{tag}z"), ins);
    };
    ins.push(abi::label(&load_fail));
    cleanup(&mut ins, "lf");
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&sign_fail));
    cleanup(&mut ins, "sf");
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&invalid_fail));
    cleanup(&mut ins, "iv");
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    cleanup(&mut ins, "af");
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}

fn verify(
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
    const LOCAL_SIZE: usize = 216;

    let load_fail = format!("{symbol}_load_fail");
    let invalid_fail = format!("{symbol}_invalid_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();

    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PUBCOLL),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MSGCOLL),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), SIGCOLL),
    ]);
    // Zero the CF object slots so the error-exit cleanup can null-guard each
    // CFRelease (the frame is not zero-initialised) — bug-55.
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PUBDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MSGDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SIGDATA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), KEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DICT),
    ]);
    emit_read_byte_list(
        symbol,
        "pub",
        PUBCOLL,
        PUBBUF,
        PUBLEN,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    // Reject a public key that is not exactly one uncompressed SEC1 point.
    // SecKeyCreateWithData validates the point too (a bad one yields NULL and
    // routes to invalid_fail), so this is a parity guard rather than a fix for a
    // live defect: the OpenSSL backend checks the length explicitly, and the two
    // backends should reject identically instead of relying on one library's
    // validation (bug-317 T4).
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), PUBLEN),
        abi::compare_immediate("%v9", &curve.point_len().to_string()),
        abi::branch_ne(&invalid_fail),
    ]);
    emit_read_byte_list(
        symbol,
        "msg",
        MSGCOLL,
        MSGBUF,
        MSGLEN,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    emit_read_byte_list(
        symbol,
        "sig",
        SIGCOLL,
        SIGBUF,
        SIGLEN,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );

    dlopen_one(
        symbol,
        SECPATH_SYMBOL,
        SEC,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlopen_one(
        symbol,
        CFPATH_SYMBOL,
        CF,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym_into(
        symbol,
        CF,
        "CFRelease",
        RELEASE,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    dlsym_into(
        symbol,
        CF,
        "CFDataCreate",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_cfdata_create(FN, PUBBUF, PUBLEN, PUBDATA, &mut ins);
    emit_cfdata_create(FN, MSGBUF, MSGLEN, MSGDATA, &mut ins);
    emit_cfdata_create(FN, SIGBUF, SIGLEN, SIGDATA, &mut ins);

    build_dict2(
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
        &mut ins,
        &mut rel,
    )?;

    dlsym_into(
        symbol,
        SEC,
        "SecKeyCreateWithData",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PUBDATA),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DICT),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    load_cf_const(
        symbol,
        SEC,
        curve.macos_algorithm(),
        ALGO,
        CONST_SCRATCH,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;

    // ok = SecKeyVerifySignature(key, algo, msgData, sigData, NULL)
    dlsym_into(
        symbol,
        SEC,
        "SecKeyVerifySignature",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), KEY),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), ALGO),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MSGDATA),
        abi::load_u64(abi::ARG[3], abi::stack_pointer(), SIGDATA),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    // Normalise the CF `Boolean` (a 0/1 byte with unspecified upper bits) to a
    // clean 0/1 by masking bit 0. A false verify sets no MFBASIC error — it is a
    // legitimate `Boolean` result.
    ins.extend([
        abi::move_immediate("%v10", "Integer", "1"),
        abi::and_registers("%v9", abi::return_register(), "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), BOOLRES),
    ]);

    cf_release(RELEASE, PUBDATA, &mut ins);
    cf_release(RELEASE, MSGDATA, &mut ins);
    cf_release(RELEASE, SIGDATA, &mut ins);
    cf_release(RELEASE, DICT, &mut ins);
    cf_release(RELEASE, KEY, &mut ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), BOOLRES),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Every error exit releases exactly the CF objects (PUBDATA/MSGDATA/SIGDATA/
    // DICT/KEY) the success path releases; slots are zero-initialised so a
    // release is a no-op until the object exists (bug-55).
    let cleanup = |ins: &mut Vec<CodeInstruction>, tag: &str| {
        cf_release_guarded(symbol, RELEASE, PUBDATA, &format!("{tag}p"), ins);
        cf_release_guarded(symbol, RELEASE, MSGDATA, &format!("{tag}m"), ins);
        cf_release_guarded(symbol, RELEASE, SIGDATA, &format!("{tag}s"), ins);
        cf_release_guarded(symbol, RELEASE, DICT, &format!("{tag}d"), ins);
        cf_release_guarded(symbol, RELEASE, KEY, &format!("{tag}k"), ins);
    };
    ins.push(abi::label(&load_fail));
    cleanup(&mut ins, "lf");
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&invalid_fail));
    cleanup(&mut ins, "iv");
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    cleanup(&mut ins, "af");
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}

/// `dst = CFDataCreate(NULL, *buf_off, *len_off)` (CFDataCreate pointer at fn_off).
fn emit_cfdata_create(
    fn_off: usize,
    buf_off: usize,
    len_off: usize,
    dst_off: usize,
    ins: &mut Vec<CodeInstruction>,
) {
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), buf_off),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), len_off),
    ]);
    call_fn(fn_off, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        dst_off,
    ));
}

/// Extract the bytes of the CFData at `data_off` into a fresh `List OF Byte` at
/// `coll_off` (via CFDataGetBytePtr/CFDataGetLength).
#[allow(clippy::too_many_arguments)]
fn emit_cfdata_to_list(
    symbol: &str,
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
    call_fn(fn_off, ins);
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
    call_fn(fn_off, ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        bytelen_off,
    ));
    emit_build_byte_list(
        symbol,
        "out",
        byteptr_off,
        bytelen_off,
        coll_off,
        alloc_fail,
        ins,
        rel,
    );
    Ok(())
}

#[cfg(test)]
mod error_path_release_tests {
    // Regression guards for bug-55: the macOS SecKey `crypto::` sign/verify/
    // generate error exits must CFRelease the SecKey/CFData/CFDictionary objects
    // the success exit releases (each null-guarded, since the slots are zeroed at
    // entry), and sign must wipe the raw private scalar. These lower and register-
    // allocate on this host; the assertions pin the guarded-release cleanup and
    // zeroing so they cannot silently regress.
    use super::*;
    use crate::target::shared::code::mir;
    use crate::target::shared::code::test_support::{has_label, TestPlatform};

    fn reloc_has(rel: &[CodeRelocation], needle: &str) -> bool {
        rel.iter().any(|r| r.to.contains(needle))
    }

    #[test]
    fn generate_releases_cf_objects_on_error() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            generate(Curve::P256, "g", &imports, &TestPlatform).expect("lower generate");
        assert!(reloc_has(&rel, "CFRelease"));
        // gen_fail null-guards each CFRelease (NUM here).
        assert!(
            has_label(&ins, "g_gfn_norel"),
            "gen_fail must null-guard CFRelease"
        );
    }

    #[test]
    fn sign_releases_cf_objects_and_wipes_scratch_on_error() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            sign(Curve::P256, "s", &imports, &TestPlatform).expect("lower sign");
        assert!(reloc_has(&rel, "CFRelease"));
        assert!(
            has_label(&ins, "s_sfp_norel"),
            "sign_fail must null-guard CFRelease(PRIVDATA)"
        );
        // The private scalar scratch is wiped on both the success (privS) and the
        // error (sfz) exits.
        assert!(
            has_label(&ins, "s_privS_noz"),
            "success exit must wipe PRIVBUF"
        );
        assert!(has_label(&ins, "s_sfz_noz"), "sign_fail must wipe PRIVBUF");
    }

    #[test]
    fn verify_releases_cf_objects_on_error() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            verify(Curve::P256, "v", &imports, &TestPlatform).expect("lower verify");
        assert!(reloc_has(&rel, "CFRelease"));
        assert!(
            has_label(&ins, "v_ivp_norel"),
            "invalid_fail must null-guard CFRelease(PUBDATA)"
        );
    }

    // bug-317 T4: the OpenSSL backend rejects a wrong-length public key with an
    // explicit length check before splicing; this backend leaned on
    // SecKeyCreateWithData to notice. Both now reject identically, per curve.
    #[test]
    fn verify_prechecks_public_key_length() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        for (curve, point_len) in [
            (Curve::P256, "65"),
            (Curve::P384, "97"),
            (Curve::P521, "133"),
        ] {
            let (_f, ins, _r, _s) =
                verify(curve, "v", &imports, &TestPlatform).expect("lower verify");
            assert!(
                // Register names are physical by this point (the body has been
                // through allocation), so the immediate is the stable signal.
                ins.iter()
                    .any(|i| i.op == CodeOp::CmpImm && i.get("rhs") == Some(point_len)),
                "verify must compare the public-key length against {point_len}"
            );
        }
    }
}
