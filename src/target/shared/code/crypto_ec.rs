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

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

/// The elliptic curve a helper targets. Only the input key size (keygen) and the
/// ECDSA message-digest algorithm (sign/verify) vary between curves.
#[derive(Clone, Copy)]
pub(super) enum Curve {
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
pub(super) fn ec_call(call: &str) -> Option<(EcOp, Curve)> {
    let (op, curve) = match call {
        "crypto.generateP256Raw" => (EcOp::Generate, Curve::P256),
        "crypto.generateP384Raw" => (EcOp::Generate, Curve::P384),
        "crypto.generateP521Raw" => (EcOp::Generate, Curve::P521),
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
pub(super) enum EcOp {
    Generate,
    Sign,
    Verify,
}

/// True for the runtime-helper symbols emitted by this module (used to gate the
/// per-platform read-only data objects).
pub(super) fn is_ec_symbol(symbol: &str) -> bool {
    symbol.starts_with("_mfb_rt_crypto_crypto_generateP")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p256")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p384")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p521")
}

pub(super) fn lower_crypto_ec_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let (op, curve) =
        ec_call(call).ok_or_else(|| format!("crypto EC helper: unknown call {call}"))?;
    if platform.target().contains("macos") {
        macos::lower(op, curve, symbol, platform_imports, platform)
    } else {
        openssl::lower(op, curve, symbol, platform_imports, platform)
    }
}

// ---------------------------------------------------------------------------
// Shared List OF Byte marshalling — the constructors are the package-neutral
// `native_helpers` emitters (bug-330); re-exported here so the two backends can
// keep importing them from their parent.
// ---------------------------------------------------------------------------

pub(super) use super::native_helpers::{emit_build_byte_list, emit_fail, emit_read_byte_list};

/// Call the function pointer stored at `fn_off` (args already in x0..). Result
/// left in the return register. Shared by both EC backends.
pub(super) fn call_fn(fn_off: usize, ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), fn_off),
        abi::branch_link_register("%v9"),
    ]);
}

pub(super) mod macos;
pub(super) mod openssl;
