//! `__crypto_ed25519PubToX25519` — shared private helper for the `crypto` package.
//!
//! Convert an Ed25519 public key to the matching X25519 (Montgomery u) public key,
//! reproducing libsodium's `crypto_sign_ed25519_pk_to_curve25519`: decode the
//! Edwards point's `y` (the low 255 bits; the sign bit is masked by
//! `__crypto_unpack25519`), then map `u = (1 + y) / (1 - y) mod 2^255-19` with the
//! shared GF(2^255-19) field ops.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed25519PubToX25519(edPub AS List OF Byte) AS List OF Byte
  LET y AS List OF Integer = __crypto_unpack25519(edPub)
  LET one AS List OF Integer = __crypto_gf1()
  LET num AS List OF Integer = __crypto_edA(one, y)
  LET den AS List OF Integer = __crypto_edZ(one, y)
  LET denInv AS List OF Integer = __crypto_inv25519(den)
  LET u AS List OF Integer = __crypto_edM(num, denInv)
  RETURN __crypto_pack25519(u)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed25519PubToX25519", BODY));
}
