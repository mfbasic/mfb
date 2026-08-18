//! `__crypto_beWord64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The big-endian 64-bit word at byte offset `o` of `data`.
FUNC __crypto_beWord64(data AS List OF Byte, o AS Integer) AS Integer
  MUT w AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 8
    LET b AS Integer = toInt(collections::get(data, o + i))
    w = bits::bor(bits::sl(w, 8), b)
    i = i + 1
  END WHILE
  RETURN w
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_beWord64", BODY));
}
