//! Linux `libcrypto` (OpenSSL) backend for the `crypto::` NIST-EC helpers.
//! Wire-compatible with the macOS `SecKey` backend (see the parent module).
//!
//! Key material is exchanged with OpenSSL as DER and driven through the `d2i_*` /
//! `EVP_DigestSign` / `EVP_DigestVerify` APIs, which are present and
//! **non-deprecated on both OpenSSL 1.1 and 3.x** (unlike the 3.x-only
//! `OSSL_PARAM`/`EVP_PKEY_fromdata` and the 3.x-deprecated `EC_KEY_*`). Every key
//! component has a fixed size per curve, so the private key is a constant PKCS#8
//! template with the scalar/point spliced at fixed offsets, and the public key is
//! a constant SPKI prefix followed by the SEC1 point.
//!
//! Keygen is the one operation without a single cross-version API: OpenSSL 3.x
//! uses `EVP_EC_gen` (non-deprecated); 1.1 falls back to `EC_KEY_*` (not
//! deprecated there). Both converge on `i2d_PrivateKey` (a stable SEC1 encoding)
//! from which the raw `0x04||X||Y||K` bytes are sliced.

use std::collections::HashMap;

use super::super::*;
use super::{emit_build_byte_list, emit_fail, emit_read_byte_list, Curve, EcOp};
use crate::arch::aarch64::abi;

const LIBCRYPTO3: &str = "libcrypto.so.3";
const LIBCRYPTO11: &str = "libcrypto.so.1.1";
const RTLD_NOW: &str = "2";
const EVP_PKEY_EC: &str = "408";

const P256_PKCS8_TMPL: &str = "308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b02010104200000000000000000000000000000000000000000000000000000000000000000a1440342000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const P256_SPKI_PREFIX: &str = "3059301306072a8648ce3d020106082a8648ce3d030107034200";
const P384_PKCS8_TMPL: &str = "3081b6020100301006072a8648ce3d020106052b8104002204819e30819b0201010430000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a16403620000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const P384_SPKI_PREFIX: &str = "3076301006072a8648ce3d020106052b81040022036200";
const P521_PKCS8_TMPL: &str = "3081ee020100301006072a8648ce3d020106052b810400230481d63081d30201010442000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a181890381860000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const P521_SPKI_PREFIX: &str = "30819b301006072a8648ce3d020106052b8104002303818600";

/// Function names resolved via dlsym.
const SYMBOLS: &[&str] = &[
    "d2i_AutoPrivateKey",
    "d2i_PUBKEY",
    "i2d_PrivateKey",
    "i2d_PUBKEY",
    "EVP_PKEY_free",
    "EVP_PKEY_new",
    "EVP_PKEY_assign",
    "EVP_MD_CTX_new",
    "EVP_MD_CTX_free",
    "EVP_sha256",
    "EVP_sha384",
    "EVP_sha512",
    "EVP_DigestSignInit",
    "EVP_DigestSign",
    "EVP_DigestVerifyInit",
    "EVP_DigestVerify",
    "EVP_EC_gen",
    "EC_KEY_new_by_curve_name",
    "EC_KEY_generate_key",
    "EC_KEY_free",
];

struct CurveParams {
    field_len: usize,
    point_len: usize,
    sec1_scalar_off: usize,
    pkcs8_len: usize,
    p8_scalar_off: usize,
    p8_point_off: usize,
    nid: &'static str,
    name: &'static str,
    tmpl_hex: &'static str,
    spki_hex: &'static str,
    digest: &'static str,
}

impl CurveParams {
    fn spki_prefix_len(&self) -> usize {
        self.spki_hex.len() / 2
    }
}

fn params(curve: Curve) -> CurveParams {
    match curve {
        Curve::P256 => CurveParams {
            field_len: 32,
            point_len: 65,
            sec1_scalar_off: 7,
            pkcs8_len: 138,
            p8_scalar_off: 36,
            p8_point_off: 73,
            nid: "415",
            name: "P-256",
            tmpl_hex: P256_PKCS8_TMPL,
            spki_hex: P256_SPKI_PREFIX,
            digest: "EVP_sha256",
        },
        Curve::P384 => CurveParams {
            field_len: 48,
            point_len: 97,
            sec1_scalar_off: 8,
            pkcs8_len: 185,
            p8_scalar_off: 35,
            p8_point_off: 88,
            nid: "715",
            name: "P-384",
            tmpl_hex: P384_PKCS8_TMPL,
            spki_hex: P384_SPKI_PREFIX,
            digest: "EVP_sha384",
        },
        Curve::P521 => CurveParams {
            field_len: 66,
            point_len: 133,
            sec1_scalar_off: 8,
            pkcs8_len: 241,
            p8_scalar_off: 35,
            p8_point_off: 108,
            nid: "716",
            name: "P-521",
            tmpl_hex: P521_PKCS8_TMPL,
            spki_hex: P521_SPKI_PREFIX,
            digest: "EVP_sha512",
        },
    }
}

fn fn_sym(name: &str) -> String {
    format!("_mfb_crypto_ec_ossl_{name}")
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

fn cstr_data(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: super::super::tls::hex_encode_cstring(text),
    }
}

pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    let mut objects = vec![
        cstr_data("_mfb_crypto_ec_lib3", LIBCRYPTO3),
        cstr_data("_mfb_crypto_ec_lib11", LIBCRYPTO11),
    ];
    for name in SYMBOLS {
        objects.push(cstr_data(&fn_sym(name), name));
    }
    for c in [Curve::P256, Curve::P384, Curve::P521] {
        let p = params(c);
        objects.push(raw_data(
            &format!("_mfb_crypto_ec_tmpl_{}", p.name),
            p.tmpl_hex,
        ));
        objects.push(raw_data(
            &format!("_mfb_crypto_ec_spki_{}", p.name),
            p.spki_hex,
        ));
        objects.push(cstr_data(
            &format!("_mfb_crypto_ec_name_{}", p.name),
            p.name,
        ));
    }
    objects
}

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

#[allow(clippy::too_many_arguments)]
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
    data_address(
        symbol,
        abi::return_register(),
        "_mfb_crypto_ec_lib3",
        ins,
        rel,
    );
    ins.push(abi::move_immediate("x1", "Integer", RTLD_NOW));
    platform.emit_libc_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&loaded),
    ]);
    data_address(
        symbol,
        abi::return_register(),
        "_mfb_crypto_ec_lib11",
        ins,
        rel,
    );
    ins.push(abi::move_immediate("x1", "Integer", RTLD_NOW));
    platform.emit_libc_call("dlopen", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::label(&loaded),
    ]);
    Ok(())
}

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
    data_address(symbol, "x1", &fn_sym(name), ins, rel);
    platform.emit_libc_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
    ]);
    Ok(())
}

/// dlsym into `dst_off`, branching to `absent` if NULL (for the optional
/// `EVP_EC_gen`, present only on OpenSSL 3.x).
#[allow(clippy::too_many_arguments)]
fn dlsym_probe(
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
    data_address(symbol, "x1", &fn_sym(name), ins, rel);
    platform.emit_libc_call("dlsym", symbol, imports, ins, rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), dst_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(absent),
    ]);
    Ok(())
}

fn call_fn(fn_off: usize, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), fn_off),
        abi::branch_link_register("%v9"),
    ]);
}

/// Copy `n` bytes from `[src_ptr_off] + src_const + [src_runtime_off]` to
/// `[dst_ptr_off] + dst_const`. Call-free (vreg scratch only).
#[allow(clippy::too_many_arguments)]
fn emit_copy(
    symbol: &str,
    tag: &str,
    src_ptr_off: usize,
    src_const: usize,
    src_runtime_off: Option<usize>,
    dst_ptr_off: usize,
    dst_const: usize,
    n: usize,
    ins: &mut Vec<CodeInstruction>,
) {
    let loop_l = format!("{symbol}_{tag}_cpy");
    let done_l = format!("{symbol}_{tag}_cpyend");
    ins.push(abi::load_u64("%v9", abi::stack_pointer(), src_ptr_off));
    if src_const > 0 {
        ins.push(abi::add_immediate("%v9", "%v9", src_const));
    }
    if let Some(off) = src_runtime_off {
        ins.extend([
            abi::load_u64("%v12", abi::stack_pointer(), off),
            abi::add_registers("%v9", "%v9", "%v12"),
        ]);
    }
    ins.push(abi::load_u64("%v10", abi::stack_pointer(), dst_ptr_off));
    if dst_const > 0 {
        ins.push(abi::add_immediate("%v10", "%v10", dst_const));
    }
    ins.extend([
        abi::move_immediate("%v11", "Integer", "0"),
        abi::move_immediate("%v13", "Integer", &n.to_string()),
        abi::label(&loop_l),
        abi::compare_registers("%v11", "%v13"),
        abi::branch_eq(&done_l),
        abi::load_u8("%v12", "%v9", 0),
        abi::store_u8("%v12", "%v10", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::add_immediate("%v11", "%v11", 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}

/// Branch to `fail` unless the byte count stored at `len_off` equals `expected`.
/// Guards the fixed-length key splices against malformed (wrong-length) input,
/// which would otherwise read past the argument buffer.
fn emit_len_check(len_off: usize, expected: usize, fail: &str, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), len_off),
        abi::compare_immediate("%v9", &expected.to_string()),
        abi::branch_ne(fail),
    ]);
}

fn emit_alloc(
    symbol: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    fail: &str,
) {
    ins.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    rel.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    ins.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail),
    ]);
}

/// Free the object at `obj_off` via `free_name` (`dlsym`d into `fn_off`) only when
/// the slot is non-NULL. The slots are zero-initialised at entry, and an object
/// is non-NULL only after libcrypto is loaded and the object created, so the
/// `dlsym` inside the guard never runs against a garbage handle. Its own dlsym
/// failure routes to `raw_fail` (a terminal fail with no further cleanup) so the
/// cleanup cannot re-enter itself. Used to make each error exit free exactly what
/// the success exit frees (bug-55).
#[allow(clippy::too_many_arguments)]
fn free_guarded(
    symbol: &str,
    handle_off: usize,
    obj_off: usize,
    free_name: &str,
    fn_off: usize,
    tag: &str,
    raw_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let skip = format!("{symbol}_{tag}_nofree");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), obj_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
    ]);
    dlsym_into(
        symbol, handle_off, free_name, fn_off, raw_fail, imports, platform, ins, rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        obj_off,
    ));
    call_fn(fn_off, ins);
    ins.push(abi::label(&skip));
    Ok(())
}

/// Overwrite the buffer at `[buf_off]` (length `[len_off]` when `Some`, else the
/// constant `len_const`) with zero, when the buffer slot is non-NULL. Wipes raw
/// EC key-material scratch (the SEC1/PKCS#8 DER and raw scalar copies) before the
/// helper returns so a later same-program arena allocation cannot be handed a
/// block still holding key bytes (bug-55). Call-free (vreg scratch only).
fn zero_guarded(
    symbol: &str,
    buf_off: usize,
    len_off: Option<usize>,
    len_const: usize,
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
    ]);
    match len_off {
        Some(off) => ins.push(abi::load_u64("%v10", abi::stack_pointer(), off)),
        None => ins.push(abi::move_immediate("%v10", "Integer", &len_const.to_string())),
    }
    ins.extend([
        abi::move_immediate("%v11", "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers("%v11", "%v10"),
        abi::branch_eq(&end_l),
        abi::store_u8("x31", "%v9", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v11", "%v11", 1),
        abi::branch(&loop_l),
        abi::label(&end_l),
        abi::label(&skip),
    ]);
}

pub(super) fn lower(
    op: EcOp,
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
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
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let p = params(curve);
    const HANDLE: usize = 0;
    const FN: usize = 8;
    const PKEY: usize = 16;
    const ECKEY: usize = 24;
    const SEC1PTR: usize = 32;
    const SEC1LEN: usize = 40;
    const SEC1PP: usize = 48;
    const RAWBUF: usize = 56;
    const RAWLEN: usize = 64;
    const COLL: usize = 80;
    const SPKIPTR: usize = 88;
    const SPKILEN: usize = 96;
    const SPKIPP: usize = 104;
    const LOCAL_SIZE: usize = 128;

    let load_fail = format!("{symbol}_load_fail");
    let gen_fail = format!("{symbol}_gen_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let raw_fail = format!("{symbol}_raw_fail");
    let eckey_path = format!("{symbol}_eckey");
    let have_pkey = format!("{symbol}_have_pkey");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();

    // Zero the pkey/eckey/scratch slots so the error-exit cleanup can null-guard
    // each free/wipe (the frame is not zero-initialised) — bug-55.
    ins.extend([
        abi::store_u64("x31", abi::stack_pointer(), PKEY),
        abi::store_u64("x31", abi::stack_pointer(), ECKEY),
        abi::store_u64("x31", abi::stack_pointer(), SEC1PTR),
    ]);

    dlopen_libcrypto(
        symbol, HANDLE, &load_fail, imports, platform, &mut ins, &mut rel,
    )?;

    dlsym_probe(
        symbol,
        HANDLE,
        "EVP_EC_gen",
        FN,
        &eckey_path,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    data_address(
        symbol,
        abi::return_register(),
        &format!("_mfb_crypto_ec_name_{}", p.name),
        &mut ins,
        &mut rel,
    );
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::branch(&have_pkey),
    ]);

    // OpenSSL 1.1: EC_KEY_new_by_curve_name + generate + EVP_PKEY_assign.
    ins.push(abi::label(&eckey_path));
    dlsym_into(
        symbol,
        HANDLE,
        "EC_KEY_new_by_curve_name",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        p.nid,
    ));
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ECKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);
    dlsym_into(
        symbol,
        HANDLE,
        "EC_KEY_generate_key",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        ECKEY,
    ));
    call_fn(FN, &mut ins);
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_PKEY_new",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        PKEY,
    ));
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_PKEY_assign",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::move_immediate("x1", "Integer", EVP_PKEY_EC),
        abi::load_u64("x2", abi::stack_pointer(), ECKEY),
    ]);
    call_fn(FN, &mut ins);
    // EVP_PKEY_assign returns 1 on success (taking ownership of eckey) and 0 on
    // failure (ownership NOT transferred). On failure eckey would leak because
    // EVP_PKEY_free no longer covers it; route to gen_fail, which EC_KEY_frees
    // the still-owned eckey. On success clear the ECKEY slot so the cleanup does
    // not EC_KEY_free a key now owned by pkey (double-free) — bug-55.
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&gen_fail),
        abi::store_u64("x31", abi::stack_pointer(), ECKEY),
    ]);

    ins.push(abi::label(&have_pkey));
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&gen_fail),
    ]);

    // len = i2d_PrivateKey(pkey, NULL)
    dlsym_into(
        symbol,
        HANDLE,
        "i2d_PrivateKey",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SEC1LEN),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&gen_fail),
        // Reload the length from its slot: the return register is not reliably
        // preserved across the compare/branch to the alloc call on x86-64.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SEC1LEN),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64("x1", abi::stack_pointer(), SEC1PTR),
        abi::store_u64("x1", abi::stack_pointer(), SEC1PP),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::add_immediate("x1", abi::stack_pointer(), SEC1PP),
    ]);
    call_fn(FN, &mut ins);

    // spkiLen = i2d_PUBKEY(pkey, NULL); buf = alloc; i2d_PUBKEY(pkey, &pp).
    // The SEC1 private encoding's public-key field is OPTIONAL (some OpenSSL
    // builds omit it), so the point is taken from the SPKI, which always carries
    // it as the trailing point_len bytes. The scalar comes from the SEC1 private.
    dlsym_into(
        symbol,
        HANDLE,
        "i2d_PUBKEY",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SPKILEN),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&gen_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SPKILEN),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64("x1", abi::stack_pointer(), SPKIPTR),
        abi::store_u64("x1", abi::stack_pointer(), SPKIPP),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::add_immediate("x1", abi::stack_pointer(), SPKIPP),
    ]);
    call_fn(FN, &mut ins);

    // raw (point_len + field_len) = point || scalar
    ins.extend([
        abi::move_immediate("%v9", "Integer", &(p.point_len + p.field_len).to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), RAWLEN),
        abi::move_register(abi::return_register(), "%v9"),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64("x1", abi::stack_pointer(), RAWBUF));
    // point = SPKI bytes after the constant-length prefix (04||X||Y follows the
    // fixed SEQ/algid/BITSTRING header directly); scalar from the SEC1 private.
    emit_copy(
        symbol,
        "pt",
        SPKIPTR,
        p.spki_prefix_len(),
        None,
        RAWBUF,
        0,
        p.point_len,
        &mut ins,
    );
    emit_copy(
        symbol,
        "sc",
        SEC1PTR,
        p.sec1_scalar_off,
        None,
        RAWBUF,
        p.point_len,
        p.field_len,
        &mut ins,
    );
    emit_build_byte_list(
        symbol,
        "out",
        RAWBUF,
        RAWLEN,
        COLL,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );

    dlsym_into(
        symbol,
        HANDLE,
        "EVP_PKEY_free",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        PKEY,
    ));
    call_fn(FN, &mut ins);
    // Wipe the SEC1 private-key DER scratch (holds the raw scalar).
    zero_guarded(symbol, SEC1PTR, Some(SEC1LEN), 0, "sec1S", &mut ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // Every error exit frees the objects (PKEY, ECKEY) the success exit frees
    // and wipes the SEC1 scratch; slots are zero-initialised so each free/wipe
    // is a no-op until it exists (bug-55).
    let cleanup = |ins: &mut Vec<CodeInstruction>,
                   rel: &mut Vec<CodeRelocation>,
                   tag: &str|
     -> Result<(), String> {
        free_guarded(
            symbol,
            HANDLE,
            ECKEY,
            "EC_KEY_free",
            FN,
            &format!("{tag}ec"),
            &raw_fail,
            imports,
            platform,
            ins,
            rel,
        )?;
        free_guarded(
            symbol,
            HANDLE,
            PKEY,
            "EVP_PKEY_free",
            FN,
            &format!("{tag}pk"),
            &raw_fail,
            imports,
            platform,
            ins,
            rel,
        )?;
        zero_guarded(symbol, SEC1PTR, Some(SEC1LEN), 0, &format!("{tag}sec1"), ins);
        Ok(())
    };
    ins.push(abi::label(&load_fail));
    cleanup(&mut ins, &mut rel, "lf")?;
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&gen_fail));
    cleanup(&mut ins, &mut rel, "gf")?;
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    cleanup(&mut ins, &mut rel, "af")?;
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    // raw_fail: a free's own dlsym failed (unreachable once libcrypto is loaded)
    // — fail without re-running cleanup.
    ins.push(abi::label(&raw_fail));
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
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
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let p = params(curve);
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
    const LOCAL_SIZE: usize = 144;

    let load_fail = format!("{symbol}_load_fail");
    let invalid_fail = format!("{symbol}_invalid_fail");
    let sign_fail = format!("{symbol}_sign_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let raw_fail = format!("{symbol}_raw_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();

    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PRIVCOLL),
        abi::store_u64("x1", abi::stack_pointer(), MSGCOLL),
    ]);
    // Zero the pkey/md-ctx and key-scratch slots so the error-exit cleanup can
    // null-guard each free/wipe (the frame is not zero-initialised) — bug-55.
    ins.extend([
        abi::store_u64("x31", abi::stack_pointer(), PKEY),
        abi::store_u64("x31", abi::stack_pointer(), MDCTX),
        abi::store_u64("x31", abi::stack_pointer(), PRIVBUF),
        abi::store_u64("x31", abi::stack_pointer(), DERBUF),
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
    emit_len_check(PRIVLEN, p.point_len + p.field_len, &invalid_fail, &mut ins);

    dlopen_libcrypto(
        symbol, HANDLE, &load_fail, imports, platform, &mut ins, &mut rel,
    )?;

    // privDer = template with point/scalar spliced from the raw key bytes.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &p.pkcs8_len.to_string()),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64("x1", abi::stack_pointer(), DERBUF),
        abi::store_u64("x1", abi::stack_pointer(), DERPP),
    ]);
    data_address(
        symbol,
        "%v9",
        &format!("_mfb_crypto_ec_tmpl_{}", p.name),
        &mut ins,
        &mut rel,
    );
    ins.push(abi::store_u64("%v9", abi::stack_pointer(), TMPLPTR));
    emit_copy(
        symbol,
        "tmpl",
        TMPLPTR,
        0,
        None,
        DERBUF,
        0,
        p.pkcs8_len,
        &mut ins,
    );
    // raw key = 0x04||X||Y||K = point(point_len) || scalar(field_len)
    emit_copy(
        symbol,
        "pt",
        PRIVBUF,
        0,
        None,
        DERBUF,
        p.p8_point_off,
        p.point_len,
        &mut ins,
    );
    emit_copy(
        symbol,
        "sc",
        PRIVBUF,
        p.point_len,
        None,
        DERBUF,
        p.p8_scalar_off,
        p.field_len,
        &mut ins,
    );

    // pkey = d2i_AutoPrivateKey(NULL, &pp, len)
    dlsym_into(
        symbol,
        HANDLE,
        "d2i_AutoPrivateKey",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), DERPP),
        abi::move_immediate("x2", "Integer", &p.pkcs8_len.to_string()),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    dlsym_into(
        symbol,
        HANDLE,
        "EVP_MD_CTX_new",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MDCTX,
    ));
    dlsym_into(
        symbol, HANDLE, p.digest, FN, &load_fail, imports, platform, &mut ins, &mut rel,
    )?;
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MD,
    ));

    // EVP_DigestSignInit(ctx, NULL, md, NULL, pkey)
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestSignInit",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::move_immediate("x1", "Integer", "0"),
        abi::load_u64("x2", abi::stack_pointer(), MD),
        abi::move_immediate("x3", "Integer", "0"),
        abi::load_u64("x4", abi::stack_pointer(), PKEY),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&sign_fail),
    ]);

    // siglen probe: EVP_DigestSign(ctx, NULL, &siglen, msg, msglen)
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestSign",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), SIGLEN),
        abi::load_u64("x3", abi::stack_pointer(), MSGBUF),
        abi::load_u64("x4", abi::stack_pointer(), MSGLEN),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&sign_fail),
    ]);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SIGLEN),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64("x1", abi::stack_pointer(), SIGBUF));
    // EVP_DigestSign(ctx, sig, &siglen, msg, msglen)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::load_u64("x1", abi::stack_pointer(), SIGBUF),
        abi::add_immediate("x2", abi::stack_pointer(), SIGLEN),
        abi::load_u64("x3", abi::stack_pointer(), MSGBUF),
        abi::load_u64("x4", abi::stack_pointer(), MSGLEN),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&sign_fail),
    ]);

    emit_build_byte_list(
        symbol,
        "out",
        SIGBUF,
        SIGLEN,
        COLL,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );

    dlsym_into(
        symbol,
        HANDLE,
        "EVP_MD_CTX_free",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MDCTX,
    ));
    call_fn(FN, &mut ins);
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_PKEY_free",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        PKEY,
    ));
    call_fn(FN, &mut ins);
    // Wipe the raw private scalar and the spliced PKCS#8 DER (both hold the key).
    zero_guarded(symbol, PRIVBUF, Some(PRIVLEN), 0, "privS", &mut ins);
    zero_guarded(symbol, DERBUF, None, p.pkcs8_len, "derS", &mut ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), COLL),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // Every error exit frees the objects (MDCTX, PKEY) the success exit frees
    // and wipes the private scratch (PRIVBUF, DERBUF); slots are zero-initialised
    // so each free/wipe is a no-op until it exists (bug-55).
    let cleanup = |ins: &mut Vec<CodeInstruction>,
                   rel: &mut Vec<CodeRelocation>,
                   tag: &str|
     -> Result<(), String> {
        free_guarded(
            symbol,
            HANDLE,
            MDCTX,
            "EVP_MD_CTX_free",
            FN,
            &format!("{tag}mc"),
            &raw_fail,
            imports,
            platform,
            ins,
            rel,
        )?;
        free_guarded(
            symbol,
            HANDLE,
            PKEY,
            "EVP_PKEY_free",
            FN,
            &format!("{tag}pk"),
            &raw_fail,
            imports,
            platform,
            ins,
            rel,
        )?;
        zero_guarded(symbol, PRIVBUF, Some(PRIVLEN), 0, &format!("{tag}pz"), ins);
        zero_guarded(symbol, DERBUF, None, p.pkcs8_len, &format!("{tag}dz"), ins);
        Ok(())
    };
    ins.push(abi::label(&load_fail));
    cleanup(&mut ins, &mut rel, "lf")?;
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&sign_fail));
    cleanup(&mut ins, &mut rel, "sf")?;
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&invalid_fail));
    cleanup(&mut ins, &mut rel, "iv")?;
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    cleanup(&mut ins, &mut rel, "af")?;
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    // raw_fail: a free's own dlsym failed (unreachable once libcrypto is loaded)
    // — fail without re-running cleanup.
    ins.push(abi::label(&raw_fail));
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
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
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let p = params(curve);
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
    const LOCAL_SIZE: usize = 160;

    let der_len = p.spki_prefix_len() + p.point_len;

    let load_fail = format!("{symbol}_load_fail");
    let invalid_fail = format!("{symbol}_invalid_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let raw_fail = format!("{symbol}_raw_fail");
    let vtrue = format!("{symbol}_vtrue");
    let vstore = format!("{symbol}_vstore");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();

    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PUBCOLL),
        abi::store_u64("x1", abi::stack_pointer(), MSGCOLL),
        abi::store_u64("x2", abi::stack_pointer(), SIGCOLL),
    ]);
    // Zero the pkey/md-ctx slots so the error-exit cleanup can null-guard each
    // free (the frame is not zero-initialised) — bug-55.
    ins.extend([
        abi::store_u64("x31", abi::stack_pointer(), PKEY),
        abi::store_u64("x31", abi::stack_pointer(), MDCTX),
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
    emit_len_check(PUBLEN, p.point_len, &invalid_fail, &mut ins);

    dlopen_libcrypto(
        symbol, HANDLE, &load_fail, imports, platform, &mut ins, &mut rel,
    )?;

    // pubDer = spki_prefix || point
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &der_len.to_string()),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64("x1", abi::stack_pointer(), DERBUF),
        abi::store_u64("x1", abi::stack_pointer(), DERPP),
    ]);
    data_address(
        symbol,
        "%v9",
        &format!("_mfb_crypto_ec_spki_{}", p.name),
        &mut ins,
        &mut rel,
    );
    ins.push(abi::store_u64("%v9", abi::stack_pointer(), PREFPTR));
    emit_copy(
        symbol,
        "pref",
        PREFPTR,
        0,
        None,
        DERBUF,
        0,
        p.spki_prefix_len(),
        &mut ins,
    );
    emit_copy(
        symbol,
        "pt",
        PUBBUF,
        0,
        None,
        DERBUF,
        p.spki_prefix_len(),
        p.point_len,
        &mut ins,
    );

    dlsym_into(
        symbol,
        HANDLE,
        "d2i_PUBKEY",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), DERPP),
        abi::move_immediate("x2", "Integer", &der_len.to_string()),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PKEY),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid_fail),
    ]);

    dlsym_into(
        symbol,
        HANDLE,
        "EVP_MD_CTX_new",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MDCTX,
    ));
    dlsym_into(
        symbol, HANDLE, p.digest, FN, &load_fail, imports, platform, &mut ins, &mut rel,
    )?;
    call_fn(FN, &mut ins);
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MD,
    ));

    dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestVerifyInit",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::move_immediate("x1", "Integer", "0"),
        abi::load_u64("x2", abi::stack_pointer(), MD),
        abi::move_immediate("x3", "Integer", "0"),
        abi::load_u64("x4", abi::stack_pointer(), PKEY),
    ]);
    call_fn(FN, &mut ins);

    // rc = EVP_DigestVerify(ctx, sig, siglen, msg, msglen); valid iff rc == 1.
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_DigestVerify",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MDCTX),
        abi::load_u64("x1", abi::stack_pointer(), SIGBUF),
        abi::load_u64("x2", abi::stack_pointer(), SIGLEN),
        abi::load_u64("x3", abi::stack_pointer(), MSGBUF),
        abi::load_u64("x4", abi::stack_pointer(), MSGLEN),
    ]);
    call_fn(FN, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_eq(&vtrue),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::branch(&vstore),
        abi::label(&vtrue),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::label(&vstore),
        abi::store_u64("%v9", abi::stack_pointer(), BOOLRES),
    ]);

    dlsym_into(
        symbol,
        HANDLE,
        "EVP_MD_CTX_free",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        MDCTX,
    ));
    call_fn(FN, &mut ins);
    dlsym_into(
        symbol,
        HANDLE,
        "EVP_PKEY_free",
        FN,
        &load_fail,
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        PKEY,
    ));
    call_fn(FN, &mut ins);

    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), BOOLRES),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // Every error exit frees the objects (MDCTX, PKEY) the success exit frees;
    // slots are zero-initialised so each free is a no-op until it exists
    // (bug-55). The public key, message, and signature buffers are not secret.
    let cleanup = |ins: &mut Vec<CodeInstruction>,
                   rel: &mut Vec<CodeRelocation>,
                   tag: &str|
     -> Result<(), String> {
        free_guarded(
            symbol,
            HANDLE,
            MDCTX,
            "EVP_MD_CTX_free",
            FN,
            &format!("{tag}mc"),
            &raw_fail,
            imports,
            platform,
            ins,
            rel,
        )?;
        free_guarded(
            symbol,
            HANDLE,
            PKEY,
            "EVP_PKEY_free",
            FN,
            &format!("{tag}pk"),
            &raw_fail,
            imports,
            platform,
            ins,
            rel,
        )?;
        Ok(())
    };
    ins.push(abi::label(&load_fail));
    cleanup(&mut ins, &mut rel, "lf")?;
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&invalid_fail));
    cleanup(&mut ins, &mut rel, "iv")?;
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    cleanup(&mut ins, &mut rel, "af")?;
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    // raw_fail: a free's own dlsym failed (unreachable once libcrypto is loaded)
    // — fail without re-running cleanup.
    ins.push(abi::label(&raw_fail));
    emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}

#[cfg(test)]
mod error_path_release_tests {
    // Regression guards for bug-55: the OpenSSL `crypto::` sign/verify/generate
    // error exits must free the EVP objects (and, for generate, EC_KEY) the
    // success exit frees, the EVP_PKEY_assign return must be checked, and the
    // private-key scratch must be wiped. These are Linux/OpenSSL-only paths that
    // cannot execute on this macOS host; the assertions pin the emitted
    // instruction stream / resolved symbols so the cleanup cannot regress.
    use super::*;
    use crate::target::shared::code::mir;
    use crate::target::shared::code::test_support::{has_label, TestPlatform};

    fn reloc_has(rel: &[CodeRelocation], needle: &str) -> bool {
        rel.iter().any(|r| r.to.contains(needle))
    }

    #[test]
    fn ec_key_free_is_emitted_as_a_data_symbol() {
        // The OpenSSL-1.1 keygen fallback needs EC_KEY_free to release an eckey
        // whose EVP_PKEY_assign failed; its name must have a C-string object.
        assert!(SYMBOLS.contains(&"EC_KEY_free"));
        assert!(data_objects()
            .iter()
            .any(|o| o.symbol == fn_sym("EC_KEY_free")));
    }

    #[test]
    fn generate_frees_pkey_and_eckey_on_error() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            generate(Curve::P256, "g", &imports, &TestPlatform).expect("lower generate");
        assert!(has_label(&ins, "g_raw_fail"), "generate needs a raw_fail terminal");
        assert!(reloc_has(&rel, "EC_KEY_free"), "gen_fail must EC_KEY_free the eckey");
        assert!(reloc_has(&rel, "EVP_PKEY_free"), "gen_fail must EVP_PKEY_free the pkey");
        // The EVP_PKEY_assign result gates a branch to gen_fail (the eckey is
        // cleared on success) — the have_pkey/gen_fail labels both exist.
        assert!(has_label(&ins, "g_gen_fail"));
        assert!(has_label(&ins, "g_have_pkey"));
    }

    #[test]
    fn sign_frees_evp_objects_on_error() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            sign(Curve::P256, "s", &imports, &TestPlatform).expect("lower sign");
        assert!(has_label(&ins, "s_raw_fail"));
        assert!(reloc_has(&rel, "EVP_MD_CTX_free"));
        assert!(reloc_has(&rel, "EVP_PKEY_free"));
        // The private scratch wipe emits guarded zero loops on the error exits.
        assert!(has_label(&ins, "s_sfpz_noz"), "sign_fail must wipe PRIVBUF");
        assert!(has_label(&ins, "s_sfdz_noz"), "sign_fail must wipe DERBUF");
    }

    #[test]
    fn verify_frees_evp_objects_on_error() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            verify(Curve::P256, "v", &imports, &TestPlatform).expect("lower verify");
        assert!(has_label(&ins, "v_raw_fail"));
        assert!(reloc_has(&rel, "EVP_MD_CTX_free"));
        assert!(reloc_has(&rel, "EVP_PKEY_free"));
    }
}
