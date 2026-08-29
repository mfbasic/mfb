//! `__crypto_ed448Verify` — shared private helper for the `crypto` package.
//!
//! RFC 8032 §5.2.7 PureEd448 verification (cofactorless, empty context): strict
//! decoding of `A` and `R` (`__crypto_ed448Decode` — canonical, on-curve, not
//! small-order), canonical `S < L`, `k = SHAKE256(dom4 ‖ R ‖ A ‖ M) mod L`, and
//! `[S]B = R + [k]A` compared through the canonical encodings with the
//! constant-time byte compare. A wrong-length key or signature is `FALSE`, never
//! an error (the Ed25519 contract).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed448Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean
  IF len(publicKey) <> 57 THEN
    RETURN FALSE
  END IF
  IF len(signature) <> 114 THEN
    RETURN FALSE
  END IF
  LET aDec AS List OF Integer = __crypto_ed448Decode(publicKey)
  IF collections::get(aDec, 0) = 0 THEN
    RETURN FALSE
  END IF
  LET bigR AS List OF Byte = __crypto_truncate(signature, 57)
  LET bigS AS List OF Byte = __crypto_slice(signature, 57, 114)
  LET rDec AS List OF Integer = __crypto_ed448Decode(bigR)
  IF collections::get(rDec, 0) = 0 THEN
    RETURN FALSE
  END IF
  IF __crypto_ed448ScalarBelowL(bigS) = FALSE THEN
    RETURN FALSE
  END IF
  MUT kInput AS List OF Byte = __crypto_concat(__crypto_concat(__crypto_ed448Dom(), bigR), publicKey)
  kInput = __crypto_concat(kInput, message)
  LET k AS List OF Byte = __crypto_ed448ModL(__crypto_bytesToLimbs(__crypto_shake256(kInput, 114)))
  LET lhs AS List OF Byte = __crypto_ed448Encode(__crypto_ed448Scalarmult(__CRYPTO_ED448_B, bigS))
  LET kA AS List OF Integer = __crypto_ed448Scalarmult(collections::mid(aDec, 1, 48), k)
  LET rhs AS List OF Byte = __crypto_ed448Encode(__crypto_ed448Add(collections::mid(rDec, 1, 48), kA))
  RETURN __crypto_constantTimeEqual(lhs, rhs)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Verify", BODY));
}
