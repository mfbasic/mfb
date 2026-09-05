//! `__crypto_convert` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::convert(conv, keys)`: dispatch on the
//! `KeyConvert` selector. `Ed25519ToX25519` converts both halves of an Ed25519
//! `KeyPair` to X25519 (private seed → X25519 scalar via
//! `__crypto_ed25519PrivToX25519`; public point → X25519 u via
//! `__crypto_ed25519PubToX25519`); `Ed448ToX448` converts an Ed448 pair (57-byte
//! seed and public key, both lengths checked) to X448 via
//! `__crypto_ed448PrivToX448` / `__crypto_ed448PubToX448`, returning the X448
//! `KeyPair`.
//!
//! bug-514. The length test alone cannot tell an Ed25519 pair from an X25519 one
//! — both halves are 32 bytes on both curves — so before either map runs, the
//! source curve is PROVEN from the pair: [`super::helper_ed25519_public`] /
//! [`super::helper_ed448_public`] re-derive the RFC 8032 public key from
//! `keys.privateKey` and it must equal `keys.publicKey`. That is a derivation
//! check, not a shape heuristic: an X25519 `u` coordinate decodes as a valid
//! Edwards `y` about half the time, so inspecting the public key alone could
//! never separate the curves, while `A = [s]B` holds for every conformant RFC
//! 8032 pair and for no X25519 pair. `convert` is the ONLY `crypto` member that
//! receives both halves of one pair, so it is the only one that can do this;
//! every member taking a lone key half takes its curve from a selector argument.
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
  IF conv = KeyConvert.Ed448ToX448 THEN
    IF len(skBytes) <> 57 OR len(pkBytes) <> 57 THEN
      FAIL error(77050002, "Ed448ToX448 requires a 57-byte Ed448 seed and public key")
    END IF
    IF __crypto_constantTimeEqual(__crypto_ed448Public(skBytes), pkBytes) = FALSE THEN
      FAIL error(77050002, "Ed448ToX448 requires an Ed448 key pair: publicKey is not the Ed448 public key of privateKey")
    END IF
    LET x448Priv AS List OF Byte = __crypto_ed448PrivToX448(skBytes)
    LET x448Pub AS List OF Byte = __crypto_ed448PubToX448(pkBytes)
    RETURN KeyPair[x448Priv, x448Pub]
  END IF
  IF len(skBytes) <> 32 OR len(pkBytes) <> 32 THEN
    FAIL error(77050002, "Ed25519ToX25519 requires a 32-byte Ed25519 seed and public key")
  END IF
  IF __crypto_constantTimeEqual(__crypto_ed25519Public(skBytes), pkBytes) = FALSE THEN
    FAIL error(77050002, "Ed25519ToX25519 requires an Ed25519 key pair: publicKey is not the Ed25519 public key of privateKey")
  END IF
  LET xPriv AS List OF Byte = __crypto_ed25519PrivToX25519(skBytes)
  LET xPub AS List OF Byte = __crypto_ed25519PubToX25519(pkBytes)
  RETURN KeyPair[xPriv, xPub]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_convert", BODY));
}
