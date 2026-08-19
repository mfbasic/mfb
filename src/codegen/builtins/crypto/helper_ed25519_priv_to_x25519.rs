//! `__crypto_ed25519PrivToX25519` — shared private helper for the `crypto` package.
//!
//! Convert an Ed25519 private key (the 32-byte seed stored as `KeyPair.privateKey`)
//! to the matching X25519 scalar, reproducing libsodium's
//! `crypto_sign_ed25519_sk_to_curve25519`: `X25519 scalar = clamp(SHA-512(seed)[0..32])`.
//! Identical to the first two steps of `__crypto_ed25519Public`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed25519PrivToX25519(seed AS List OF Byte) AS List OF Byte
  LET d AS List OF Byte = __crypto_sha512_bytes(seed)
  RETURN __crypto_clampScalar(__crypto_truncate(d, 32))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed25519PrivToX25519", BODY));
}
