//! `__crypto_clampScalar448` — shared private helper for the `crypto` package.
//!
//! RFC 7748 §5 `decodeScalar448`: clear the two low bits of byte 0 and set bit 7
//! of byte 55 (the cofactor-clearing / fixed-top-bit clamp), on a 56-byte scalar.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 7748 decodeScalar448 clamp: k[0] &= 252; k[55] |= 128.
FUNC __crypto_clampScalar448(a AS List OF Byte) AS List OF Byte
  MUT r AS List OF Byte = a
  r = collections::set(r, 0, toByte(bits::band(toInt(collections::get(r, 0)), 252)))
  r = collections::set(r, 55, toByte(bits::bor(toInt(collections::get(r, 55)), 128)))
  RETURN r
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_clampScalar448", BODY));
}
