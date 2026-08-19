//! `__crypto_convert` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::convert(conv, keys)`: dispatch on the
//! `KeyConvert` selector and, for `Ed25519ToX25519`, convert both halves of an
//! Ed25519 `KeyPair` to X25519 (private seed → X25519 scalar via
//! `__crypto_ed25519PrivToX25519`; public point → X25519 u via
//! `__crypto_ed25519PubToX25519`), returning the X25519 `KeyPair`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_convert(conv AS KeyConvert, keys AS KeyPair) AS KeyPair
  LET skBytes AS List OF Byte = keys.privateKey
  LET pkBytes AS List OF Byte = keys.publicKey
  LET xPriv AS List OF Byte = __crypto_ed25519PrivToX25519(skBytes)
  LET xPub AS List OF Byte = __crypto_ed25519PubToX25519(pkBytes)
  RETURN KeyPair[xPriv, xPub]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_convert", BODY));
}
