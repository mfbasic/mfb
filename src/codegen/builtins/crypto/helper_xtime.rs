//! `__crypto_xtime` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GF(2^8) multiply-by-2 (xtime) for MixColumns.
FUNC __crypto_xtime(b AS Integer) AS Integer
  LET shifted AS Integer = bits::sl(b, 1)
  IF bits::band(b, 128) <> 0 THEN
    RETURN bits::band(bits::bxor(shifted, 27), 255)
  END IF
  RETURN bits::band(shifted, 255)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_xtime", BODY));
}
