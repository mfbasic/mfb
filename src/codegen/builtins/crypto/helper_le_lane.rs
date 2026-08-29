//! `__crypto_leLane` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The little-endian 64-bit lane at byte offset `o` of `data` (a raw bit pattern).
FUNC __crypto_leLane(data AS List OF Byte, o AS Integer) AS Integer
  MUT lane AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 8
    LET b AS Integer = toInt(collections::get(data, o + i))
    lane = bits::bor(lane, bits::sl(b, i * 8))
    i = i + 1
  END WHILE
  RETURN lane
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_leLane", BODY));
}
