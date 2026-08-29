//! `__crypto_ed448PubToX448` — shared private helper for the `crypto` package.
//!
//! Convert an Ed448 public key (57 bytes: the 56-byte little-endian Edwards `y`
//! plus the sign byte) to the matching X448 public key by the RFC 7748 §4.2
//! 4-isogeny from edwards448 to curve448, `u = y² / x²`. On edwards448
//! (`x² + y² = 1 + d·x²·y²`, `d = −39081`) that is `u = y²·(1 − d·y²) / (1 − y²)
//! = y²·(1 + 39081·y²) / (1 − y²)`, so only `y` is needed and no square root is
//! taken — the same formula libdecaf's `decaf_ed448_convert_public_key_to_x448`
//! uses. The map sends the edwards448 base point to `u = 5`, so a converted pair
//! satisfies `X448(convertedPrivate, 5) = convertedPublic` (pinned by the
//! `crypto-x448-valid` fixture against an OpenSSL-backed oracle).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Ed448 public (57 bytes) -> X448 public: the RFC 7748 §4.2 4-isogeny u = y^2/x^2.
FUNC __crypto_ed448PubToX448(edPub AS List OF Byte) AS List OF Byte
  LET y AS List OF Integer = __crypto_gf448Unpack(collections::mid(edPub, 0, 56))
  LET one AS List OF Integer = __crypto_gf448One()
  LET y2 AS List OF Integer = __crypto_gf448Mul(y, y)
  LET num AS List OF Integer = __crypto_gf448Mul(y2, __crypto_gf448Add(one, __crypto_gf448MulSmall(y2, 39081)))
  LET den AS List OF Integer = __crypto_gf448Sub(one, y2)
  RETURN __crypto_gf448Pack(__crypto_gf448Mul(num, __crypto_gf448Inv(den)))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448PubToX448", BODY));
}
