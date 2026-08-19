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

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::*;
use crate::codegen::memory::marshal::*;
use std::collections::HashMap;

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

fn wide_addr(
    from: &str,
    dst: impl Into<Operand>,
    id: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
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
    platform.emit_external_call(symbol, from, imports, ins, rel)?;
    ins.push(abi::add_stack(frame));
    // bug-446/bug-447: a Win64 external call returns its NTSTATUS in `rax`
    // (`c_return(0)`), and Win64 `emit_external_call` does NOT stage it into the
    // aligned MFB result bank — on Win64 `return_register()` (`mfb_return(0)`) is
    // `rcx`, which still holds the call's first argument (e.g. the `hKey` handle).
    // Sign-extend the 32-bit status from `c_return(0)` INTO `return_register()` so
    // every caller's `status < 0` / `status == 0` check reads the real NTSTATUS.
    // (`p*Sign`/`generate` only survived the old `rcx`-reading bug by accident: a
    // positive stale pointer is never `< 0`; `p*Verify`'s `== 0` check always
    // failed, which was bug-446.)
    ins.push(abi::sign_extend_word(
        abi::return_register(),
        abi::c_return(0),
    ));
    Ok(())
}

/// Minted scratch-vreg palette for one emitted CNG helper. Each field is a fresh
/// vreg the shared allocator colors; one `Sc` is built per lowering entry
/// (`generate`/`sign`/`verify`) and threaded into every emitter contributing to
/// that function, so all uses of a given original hand-picked number map to the
/// same minted vreg (and the internal `%v4`/`%v5` copy scratch stays distinct from
/// the `%v6..%v15` the callers use). Fields a given helper does not use are simply
/// never emitted.
struct Sc {
    v4: String,
    v5: String,
    v6: String,
    v7: String,
    v8: String,
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
        Sc {
            v4: vregs.next(),
            v5: vregs.next(),
            v6: vregs.next(),
            v7: vregs.next(),
            v8: vregs.next(),
            v9: vregs.next(),
            v10: vregs.next(),
            v11: vregs.next(),
            v12: vregs.next(),
            v13: vregs.next(),
            v14: vregs.next(),
            v15: vregs.next(),
        }
    }
}

/// A copy loop: `count` bytes from `[src]` to `[dst]` (both register operands,
/// consumed). Uses `%v9`/`%v-tmp` scratch named by `tag`.
fn copy_bytes(
    sc: &Sc,
    src: &str,
    dst: &str,
    count: &str,
    tag: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    // Internal scratch sc.v4/sc.v5 must not alias any caller's src/dst/count
    // (callers use sc.v6..sc.v15) — otherwise `load,[ptr]` would clobber a pointer.
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

pub(crate) fn lower(
    op: EcOp,
    curve: Curve,
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match op {
        EcOp::Sign => sign(curve, symbol, imports, platform),
        EcOp::Verify => verify(curve, symbol, imports, platform),
    }
}

/// Destroy `hKey` (at `hkey_off`) and close `hAlg` (at `halg_off`), each null-guarded.
#[allow(clippy::too_many_arguments)]
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

include!("cng_sign_verify.rs");

#[cfg(test)]
mod cng_backend_tests {
    // Regression guards for bug-415: the Windows CNG EC verify backend must (1)
    // raise ErrInvalidArgument (not a FALSE verdict / ErrUnknown) on a malformed
    // public key, matching the macOS/OpenSSL backends; (2) bound its hand-rolled
    // DER signature parse against SIGLEN so an untrusted short signature cannot
    // read past the arena buffer; and (3) close the BCryptHash provider handle on
    // the hash-failure path instead of leaking it. These are Windows-CNG-only
    // paths that cannot execute on this macOS host — the assertions pin the
    // emitted instruction stream / resolved symbols so the fixes cannot regress.
    use super::*;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::tests::{has_label, TestPlatform};

    fn reloc_has(rel: &[CodeRelocation], needle: &str) -> bool {
        rel.iter().any(|r| r.to.contains(needle))
    }

    fn lower_verify() -> (Vec<CodeInstruction>, Vec<CodeRelocation>) {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            verify(Curve::P256, "v", &imports, &TestPlatform).expect("lower verify");
        (ins, rel)
    }

    #[test]
    fn verify_raises_invalid_argument_on_malformed_key() {
        let (ins, rel) = lower_verify();
        // A dedicated ErrInvalidArgument exit exists and is wired to the argument
        // error message (bug-415 defect 1). Before the fix `verify` had no
        // ERR_INVALID_ARGUMENT path at all — a wrong-length key returned FALSE.
        assert!(
            has_label(&ins, "v_invalid"),
            "verify must have an ErrInvalidArgument exit for a malformed key"
        );
        assert!(
            reloc_has(&rel, "_mfb_str_error_invalid_argument"),
            "verify's invalid exit must reference the argument-error message"
        );
    }

    #[test]
    fn verify_bounds_der_parse_against_siglen() {
        // The bounded parser (bug-415 defect 2) routes every DER length that would
        // read past the signature buffer to a dedicated out-of-bounds guard that
        // falls through to the FALSE verdict; the unbounded parser had no such
        // guard. `v_oob` is a real branch target reached from the SEQUENCE-header
        // length check and both der_decode_int bounds checks.
        let (ins, _rel) = lower_verify();
        assert!(
            has_label(&ins, "v_oob"),
            "verify must bound the DER parse against SIGLEN via an out-of-bounds guard"
        );
    }

    #[test]
    fn verify_closes_hash_provider_on_hash_failure() {
        // BCryptHash failure must route through a dedicated path that closes the
        // hash provider before failing, rather than branching straight to the
        // shared fail exit and leaking the handle (bug-415 defect 3).
        let (ins, _rel) = lower_verify();
        assert!(
            has_label(&ins, "v_hashfail"),
            "hash_message must close the provider on the BCryptHash failure path"
        );
    }
}
