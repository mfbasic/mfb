//! `__crypto_rand62` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A uniform 62-bit value from `crypto::randomBytes` (7 full bytes + 6 bits).
' Bounded below 2^62 so it stays a non-negative `Integer`.
FUNC __crypto_rand62() AS Integer
  LET rb AS List OF Byte = crypto::randomBytes(8)
  MUT v AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 7
    v = v * 256 + toInt(collections::get(rb, i))
    i = i + 1
  END WHILE
  LET top AS Integer = bits::band(toInt(collections::get(rb, 7)), 63)
  RETURN v + top * 72057594037927936
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_rand62", BODY));
}
