//! `__crypto_sha1F` — shared private helper for the `crypto` package.
//!
//! The SHA-1 round function `f_t(b, c, d)` (FIPS 180-4 §4.1.1): `Ch` for rounds
//! 0–19, `Parity` for 20–39, `Maj` for 40–59, `Parity` again for 60–79. The
//! selection depends only on the public round counter `t`, never on the words.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' SHA-1 round function f_t: Ch (0..19), Parity (20..39), Maj (40..59), Parity (60..79).
FUNC __crypto_sha1F(t AS Integer, b AS Integer, c AS Integer, d AS Integer) AS Integer
  IF t < 20 THEN
    RETURN __crypto_ch32(b, c, d)
  END IF
  IF t < 40 THEN
    RETURN bits::bxor(bits::bxor(b, c), d)
  END IF
  IF t < 60 THEN
    RETURN __crypto_maj32(b, c, d)
  END IF
  RETURN bits::bxor(bits::bxor(b, c), d)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha1F", BODY));
}
