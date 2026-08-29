//! `__crypto_ed448Public` — shared private helper for the `crypto` package.
//!
//! RFC 8032 §5.2.5 key generation: `h = SHAKE256(seed, 114)`, `s = prune(h[0..57])`,
//! `A = [s]B` encoded.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed448Public(seed AS List OF Byte) AS List OF Byte
  LET h AS List OF Byte = __crypto_shake256(seed, 114)
  LET s AS List OF Byte = __crypto_ed448Prune(__crypto_truncate(h, 57))
  RETURN __crypto_ed448Encode(__crypto_ed448Scalarmult(__CRYPTO_ED448_B, s))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Public", BODY));
}
