//! `__crypto_clampScalar` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_clampScalar(a AS List OF Byte) AS List OF Byte
  MUT r AS List OF Byte = a
  r = collections::set(r, 0, toByte(bits::band(toInt(collections::get(r, 0)), 248)))
  LET hi AS Integer = bits::bor(bits::band(toInt(collections::get(r, 31)), 127), 64)
  r = collections::set(r, 31, toByte(hi))
  RETURN r
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_clampScalar", BODY));
}
