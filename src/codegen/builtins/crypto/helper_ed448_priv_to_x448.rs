//! `__crypto_ed448PrivToX448` — shared private helper for the `crypto` package.
//!
//! Convert an Ed448 private key (the 57-byte seed) to the matching X448 scalar:
//! the first 56 bytes of `SHAKE256(seed, 114)` — the Ed448 secret-scalar bytes of
//! RFC 8032 §5.2.5 before pruning. RFC 8032's pruning (clear bits 0–1 of byte 0,
//! set bit 7 of byte 55, zero byte 56) and RFC 7748's `decodeScalar448` clamp
//! agree on those 56 bytes, so X448 clamping the result reproduces the exact
//! Ed448 scalar, and `X448(result, 5)` equals the isogeny image of the Ed448
//! public key. This is libdecaf's `decaf_ed448_convert_private_key_to_x448`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Ed448 seed (57 bytes) -> X448 private: SHAKE256(seed)[0..56].
FUNC __crypto_ed448PrivToX448(seed AS List OF Byte) AS List OF Byte
  RETURN __crypto_shake256(seed, 56)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448PrivToX448", BODY));
}
