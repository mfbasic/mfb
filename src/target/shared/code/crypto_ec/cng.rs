//! Windows CNG/BCrypt backend for the `crypto::` NIST-EC helpers. CNG is linked
//! through the IAT (no dlopen/dlsym bridge), so this backend is a straight
//! sequence of `BCrypt*` calls plus the fixed-format blob/DER conversions the
//! wire-compatible encodings require (see the parent module):
//!
//!   * private key = `0x04 ‖ X ‖ Y ‖ K`   (SEC1 point ‖ scalar)
//!   * public key  = `0x04 ‖ X ‖ Y`
//!   * signature   = ASN.1 DER `Ecdsa-Sig-Value`  (SEQUENCE{INTEGER r, INTEGER s})
//!
//! CNG's own key blob is `BCRYPT_ECCKEY_BLOB { ULONG dwMagic; ULONG cbKey; }`
//! followed by `X‖Y` (public) or `X‖Y‖d` (private), all big-endian `cbKey`-wide.
//! `BCryptSignHash` emits a fixed `r‖s` (2·field); this backend DER-encodes it and
//! DER-decodes the peer signature back into `r‖s` for `BCryptVerifySignature`.

use std::collections::HashMap;

use super::super::native_helpers::{emit_data_address, emit_zero_guarded};
use super::super::*;
use super::{emit_build_byte_list, emit_fail, emit_read_byte_list, Curve, EcOp};
use crate::target::shared::abi;


impl Curve {
    fn field_len(self) -> usize {
        match self {
            Curve::P256 => 32,
            Curve::P384 => 48,
            Curve::P521 => 66,
        }
    }
    fn algo_id(self) -> &'static str {
        match self {
            Curve::P256 => "ECDSA_P256",
            Curve::P384 => "ECDSA_P384",
            Curve::P521 => "ECDSA_P521",
        }
    }
    fn hash_id(self) -> &'static str {
        match self {
            Curve::P256 => "SHA256",
            Curve::P384 => "SHA384",
            Curve::P521 => "SHA512",
        }
    }
    fn hash_len(self) -> usize {
        match self {
            Curve::P256 => 32,
            Curve::P384 => 48,
            Curve::P521 => 64,
        }
    }
    fn priv_magic(self) -> &'static str {
        match self {
            Curve::P256 => "844317509", // 0x32534345 'ECS2'
            Curve::P384 => "877871941", // 0x34534345 'ECS4'
            Curve::P521 => "911426373", // 0x36534345 'ECS6'
        }
    }
    fn pub_magic(self) -> &'static str {
        match self {
            Curve::P256 => "827540293", // 0x31534345 'ECS1'
            Curve::P384 => "861094725", // 0x33534345 'ECS3'
            Curve::P521 => "894649157", // 0x35534345 'ECS5'
        }
    }
}

fn sym(name: &str) -> String {
    format!("_mfb_crypto_ec_w_{name}")
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

const WIDE_IDS: &[&str] = &[
    "ECDSA_P256",
    "ECDSA_P384",
    "ECDSA_P521",
    "SHA256",
    "SHA384",
    "SHA512",
    "ECCPRIVATEBLOB",
    "ECCPUBLICBLOB",
];

pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    WIDE_IDS.iter().map(|id| wide_cstr(&sym(id), id)).collect()
}

fn wide_addr(from: &str, dst: &str, id: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>) {
    emit_data_address(from, dst, &sym(id), ins, rel);
}

/// Emit a Win64 external `BCrypt*` call: args 0..=3 preloaded in
/// `return_register`/`ARG[1..3]`, args 4.. in `ARG[4]`.. spilled to the stack tail
/// above the shadow space (bug-384). Sign-extends the NTSTATUS return (`< 0` fails).
fn bcrypt_call(
    from: &str,
    symbol: &str,
    n_args: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if n_args > 4 {
        let stack = n_args - 4;
        let frame = (0x20 + stack * 8 + 15) & !15;
        ins.push(abi::subtract_stack(frame));
        for i in 0..stack {
            ins.push(abi::store_u64(abi::ARG[4 + i], abi::stack_pointer(), 0x20 + i * 8));
        }
        platform.emit_libc_call(symbol, from, imports, ins, rel)?;
        ins.push(abi::add_stack(frame));
    } else {
        platform.emit_libc_call(symbol, from, imports, ins, rel)?;
    }
    ins.push(abi::sign_extend_word(abi::return_register(), abi::return_register()));
    Ok(())
}

/// A copy loop: `count` bytes from `[src]` to `[dst]` (both register operands,
/// consumed). Uses `%v9`/`%v-tmp` scratch named by `tag`.
fn copy_bytes(src: &str, dst: &str, count: &str, tag: &str, ins: &mut Vec<CodeInstruction>) {
    // Internal scratch %v4/%v5 must not alias any caller's src/dst/count (callers
    // use %v6..%v15) — otherwise `load %v9,[%v9]` would clobber a %v9 pointer.
    let loop_l = format!("{tag}_cp");
    let done_l = format!("{tag}_cpd");
    ins.extend([
        abi::move_immediate("%v4", "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers("%v4", count),
        abi::branch_eq(&done_l),
        abi::load_u8("%v5", src, 0),
        abi::store_u8("%v5", dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::add_immediate("%v4", "%v4", 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
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

// ---------------------------------------------------------------------------
// generate
// ---------------------------------------------------------------------------
fn generate(
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let field = curve.field_len();
    let raw_len = 1 + 3 * field;
    const HALG: usize = 0;
    const HKEY: usize = 8;
    const BLOB: usize = 16;
    const CBRES: usize = 24;
    const RAW: usize = 32;
    const RAWLEN: usize = 40;
    const COLL: usize = 48;
    const BLOBCAP: usize = 8 + 3 * 66;
    const LOCAL_SIZE: usize = 64;

    let fail = format!("{symbol}_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let cleanup = format!("{symbol}_cleanup");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HALG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HKEY),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BLOB),
    ]);

    ins.push(abi::add_immediate(abi::return_register(), abi::stack_pointer(), HALG));
    wide_addr(symbol, abi::ARG[1], curve.algo_id(), &mut ins, &mut rel);
    ins.extend([
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptOpenAlgorithmProvider", 4, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));

    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HALG),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptGenerateKeyPair", 4, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));

    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptFinalizeKeyPair", 2, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));

    // Allocate blob buffer + raw-key output buffer.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &BLOBCAP.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), BLOB));
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &raw_len.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), RAW),
        abi::move_immediate("%v9", "Integer", &raw_len.to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), RAWLEN),
    ]);

    // BCryptExportKey(hKey, NULL, L"ECCPRIVATEBLOB", blob, BLOBCAP, &cbResult, 0)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HKEY),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    wide_addr(symbol, abi::ARG[2], "ECCPRIVATEBLOB", &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(abi::ARG[3], abi::stack_pointer(), BLOB),
        abi::move_immediate(abi::ARG[4], "Integer", &BLOBCAP.to_string()),
        abi::add_immediate(abi::ARG[5], abi::stack_pointer(), CBRES),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptExportKey", 7, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&fail));
    let _ = CBRES;

    // raw = 0x04 ‖ (blob body X‖Y‖d). Blob body starts at header +8.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), RAW),
        abi::move_immediate("%v9", "Byte", "4"),
        abi::store_u8("%v9", "%v10", 0),
        abi::load_u64("%v11", abi::stack_pointer(), BLOB),
        abi::add_immediate("%v11", "%v11", 8),
        abi::add_immediate("%v12", "%v10", 1),
        abi::move_immediate("%v13", "Integer", &(3 * field).to_string()),
    ]);
    copy_bytes("%v11", "%v12", "%v13", &format!("{symbol}_gk"), &mut ins);

    // Clean up the CNG handles and wipe the private blob BEFORE building the
    // result — the cleanup calls clobber the caller-saved result registers, and
    // `emit_build_byte_list` sets them last. `emit_cleanup` nulls the handle slots
    // so the shared error labels below can reuse it idempotently.
    emit_cleanup(symbol, "c1", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_zero_guarded(symbol, BLOB, None, BLOBCAP, "blobz", &mut ins);
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_out_loop"),
        &format!("{symbol}_out_done"),
        RAW,
        RAWLEN,
        Some(COLL),
        abi::RET[1],
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::branch(&done));

    ins.push(abi::label(&fail));
    emit_cleanup(symbol, "c2", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_UNKNOWN_CODE, ERR_UNKNOWN_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_cleanup(symbol, "c3", HKEY, HALG, imports, platform, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);

    let _ = cleanup;
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    Ok((frame, ins, rel, slots))
}

/// Destroy `hKey` (at `hkey_off`) and close `hAlg` (at `halg_off`), each null-guarded.
fn emit_cleanup(
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
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), hkey_off)); // null: cleanup is idempotent
    ins.push(abi::label(&no_key));
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), halg_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_alg),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    bcrypt_call(symbol, "BCryptCloseAlgorithmProvider", 2, imports, platform, ins, rel)?;
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), halg_off));
    ins.push(abi::label(&no_alg));
    Ok(())
}

include!("cng_sign_verify.rs");
