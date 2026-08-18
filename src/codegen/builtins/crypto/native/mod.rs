//! Native code generation for the `crypto::` NIST-EC public-key helpers
//! (plan-04-crypto.md Part C). The elliptic-curve operations are bound to the
//! platform's modern key API rather than implemented as software cores: generic
//! NIST bignum arithmetic is ~100x costlier than Ed25519's special-prime field
//! and is impractical over the package's `bits::` layer.
//!
//!   * macOS  — `SecKey` (Security.framework) + CoreFoundation, dlopen/dlsym.
//!   * Linux  — `EVP_PKEY` (libcrypto) via dlopen/dlsym (see `crypto_ec_openssl`).
//!
//! The two backends are wire-compatible (the user's hard requirement): a key or
//! signature produced on one platform is accepted by the other. The agreed
//! encodings are
//!
//!   * private key = `0x04 ‖ X ‖ Y ‖ K`  (SEC1 uncompressed point followed by the
//!     big-endian scalar) — self-contained so every backend can reconstruct the
//!     key without deriving the public point;
//!   * public key  = `0x04 ‖ X ‖ Y`      (SEC1 uncompressed point);
//!   * signature   = ASN.1 DER `Ecdsa-Sig-Value` (X9.62).
//!
//! Field width per curve: P-256 → 32, P-384 → 48, P-521 → 66 bytes.
//!
//! Native helpers only ever return a `List OF Byte` (raw key bytes / DER
//! signature) or a `Boolean` (verify) — never a record. `crypto::generateP*`
//! is source glue that calls the raw-keygen helper and slices the public point
//! out of the private bytes to build the `KeyPair` (see `crypto_package.mfb`).

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::registry::OsLowerCtx;
use crate::target::shared::abi;
use std::collections::HashMap;
#[derive(Clone, Copy)]
pub(crate) enum Curve {
    P256,
    P384,
    P521,
}

impl Curve {
    /// Length in bytes of an uncompressed SEC1 public point (`04 || X || Y`).
    ///
    /// Both backends splice fixed-length keys, so both must reject a public key
    /// of any other length up front. The OpenSSL backend carries this in its
    /// `CurveParams`; it lives here so the macOS backend's parity check (bug-317
    /// T4) cannot drift from it.
    fn point_len(self) -> usize {
        match self {
            Curve::P256 => 65,
            Curve::P384 => 97,
            Curve::P521 => 133,
        }
    }
    // `bits` and `macos_algorithm` are macOS-only inputs and live with the macOS
    // backend (bug-330); the OpenSSL backend carries its own curve table.
}

/// Map a runtime-helper call name onto (operation, curve).
pub(crate) fn ec_call(call: &str) -> Option<(EcOp, Curve)> {
    let (op, curve) = match call {
        // Each `generateP*` is a single public native member (its raw twin was
        // collapsed in); its helper builds the `KeyPair` record directly.
        "crypto.generateP256" => (EcOp::Generate, Curve::P256),
        "crypto.generateP384" => (EcOp::Generate, Curve::P384),
        "crypto.generateP521" => (EcOp::Generate, Curve::P521),
        "crypto.p256Sign" => (EcOp::Sign, Curve::P256),
        "crypto.p384Sign" => (EcOp::Sign, Curve::P384),
        "crypto.p521Sign" => (EcOp::Sign, Curve::P521),
        "crypto.p256Verify" => (EcOp::Verify, Curve::P256),
        "crypto.p384Verify" => (EcOp::Verify, Curve::P384),
        "crypto.p521Verify" => (EcOp::Verify, Curve::P521),
        _ => return None,
    };
    Some((op, curve))
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum EcOp {
    Generate,
    Sign,
    Verify,
}

/// True for the runtime-helper symbols emitted by this module (used to gate the
/// per-platform read-only data objects).
pub(crate) fn is_ec_symbol(symbol: &str) -> bool {
    symbol.starts_with("_mfb_rt_crypto_crypto_generateP")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p256")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p384")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p521")
}

pub(crate) fn lower_crypto_ec_helper(
    call: &str,
    symbol: &str,
    keypair: Option<&TypeModel>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let (op, curve) =
        ec_call(call).ok_or_else(|| format!("crypto EC helper: unknown call {call}"))?;
    match platform.family() {
        PlatformFamily::MacOS => {
            macos::lower(op, curve, symbol, keypair, platform_imports, platform)
        }
        PlatformFamily::Linux => {
            openssl::lower(op, curve, symbol, keypair, platform_imports, platform)
        }
        // The Windows EC backend is CNG/BCrypt (plan-47-J), linked through the IAT.
        PlatformFamily::Windows => {
            cng::lower(op, curve, symbol, keypair, platform_imports, platform)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared List OF Byte marshalling — the constructors are the package-neutral
// `native_helpers` emitters (bug-330); re-exported here so the two backends can
// keep importing them from their parent.
// ---------------------------------------------------------------------------

pub(crate) use crate::codegen::error::constants::{
    RESULT_OK_TAG, RESULT_TAG_REGISTER, RESULT_VALUE_REGISTER,
};
pub(crate) use crate::codegen::error::emission::emit_fail;
pub(crate) use crate::codegen::memory::marshal::{
    emit_build_byte_list, emit_build_inlined_record, emit_read_byte_list, RecordBuildScratch,
};

/// Call the function pointer stored at `fn_off` (args already in x0..). Result
/// left in the return register. Shared by both EC backends.
pub(crate) fn call_fn(fn_off: usize, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), fn_off),
        abi::branch_link_register("%v9"),
    ]);
}

/// Build the SEC1 public-key `List OF Byte` — the first `curve.point_len()` bytes
/// of the raw `0x04||X||Y||K` buffer at `raw_ptr_slot` (`0x04||X||Y`) — into
/// `pub_coll_slot`, using `pub_len_slot` as scratch. Called while the raw buffer
/// is still live (on macOS it aliases the CFData, which must not yet be released).
/// Branches to `alloc_fail` on failure.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_build_pub_list(
    symbol: &str,
    curve: Curve,
    raw_ptr_slot: usize,
    pub_len_slot: usize,
    pub_coll_slot: usize,
    alloc_fail: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.extend([
        abi::move_immediate("%v9", "Integer", &curve.point_len().to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), pub_len_slot),
    ]);
    emit_build_byte_list(
        symbol,
        &format!("{symbol}_pub_build_loop"),
        &format!("{symbol}_pub_build_done"),
        raw_ptr_slot,
        pub_len_slot,
        Some(pub_coll_slot),
        abi::mfb_return(1),
        alloc_fail,
        ins,
        rel,
    );
}

/// Assemble the `crypto::KeyPair` record from the already-built `privateKey`
/// (`priv_coll_slot`) and `publicKey` (`pub_coll_slot`) `List OF Byte` blocks,
/// via the generic spec-canonical record marshaller, and leave it as the fallible
/// success value (`RESULT_VALUE_REGISTER` = record pointer, `RESULT_TAG_REGISTER`
/// = `RESULT_OK_TAG`). Both fields are inlined into the record's data region
/// (`List OF Byte` is a flat composite), so the whole `KeyPair` is a single
/// pointer-free block — byte-for-byte the image the former `KeyPair[priv, pub]`
/// source glue produced. Set last so no later call clobbers the result registers.
/// Branches to `alloc_fail` on failure.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_build_keypair_record(
    symbol: &str,
    type_model: &TypeModel,
    priv_coll_slot: usize,
    pub_coll_slot: usize,
    scratch: &RecordBuildScratch,
    alloc_fail: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_build_inlined_record(
        symbol,
        "kp",
        "KeyPair",
        type_model,
        &[priv_coll_slot, pub_coll_slot],
        scratch,
        RESULT_VALUE_REGISTER,
        alloc_fail,
        ins,
        rel,
    )?;
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}

pub(crate) mod cng;
pub(crate) mod macos;
pub(crate) mod openssl;
mod random;

/// OsLower-shaped entry for `crypto::randomBytes` — the CSPRNG runtime helper.
/// The per-compilation [`OsLowerCtx`] carries no state this helper needs; it is
/// dispatched generically through `registry::os_helper` (`crypto/mod.rs`'s
/// `Body::native` slots point here) exactly like `os`/`fs`/`io`.
pub(crate) fn lower_crypto_random_bytes(
    _call: &str,
    symbol: &str,
    _ctx: &OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    random::lower_crypto_random_bytes_helper(symbol, platform_imports, platform)
}

/// OsLower-shaped entry for the NIST-EC public-key helpers (`crypto::generateP{256,
/// 384,521}`, `crypto::p{256,384,521}{Sign,Verify}`). `call` selects the (operation,
/// curve); the per-backend emission (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG)
/// is chosen by `platform.family()`.
///
/// The `crypto::generateP*` members **return a record**: their helper builds the
/// `KeyPair` from the generated bytes, so it needs the record's field layout — the
/// [`OsLowerCtx::type_model`]. Every other EC call returns a `List OF Byte` (or
/// `Boolean`) and passes `None`.
pub(crate) fn lower_crypto_ec(
    call: &str,
    symbol: &str,
    ctx: &OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let keypair = matches!(
        call,
        "crypto.generateP256" | "crypto.generateP384" | "crypto.generateP521"
    )
    .then_some(ctx.type_model);
    lower_crypto_ec_helper(call, symbol, keypair, platform_imports, platform)
}
