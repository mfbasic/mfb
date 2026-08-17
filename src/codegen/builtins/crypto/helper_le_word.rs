//! `__crypto_leWord` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The little-endian 32-bit word at byte offset `o` of `data`.
FUNC __crypto_leWord(data AS List OF Byte, o AS Integer) AS Integer
  LET b0 AS Integer = toInt(collections::get(data, o))
  LET b1 AS Integer = toInt(collections::get(data, o + 1))
  LET b2 AS Integer = toInt(collections::get(data, o + 2))
  LET b3 AS Integer = toInt(collections::get(data, o + 3))
  LET w1 AS Integer = bits::sl(b1, 8)
  LET w2 AS Integer = bits::sl(b2, 16)
  LET w3 AS Integer = bits::sl(b3, 24)
  LET lo AS Integer = bits::bor(b0, w1)
  LET hi AS Integer = bits::bor(w2, w3)
  RETURN bits::bor(lo, hi)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_leWord", BODY));
}
