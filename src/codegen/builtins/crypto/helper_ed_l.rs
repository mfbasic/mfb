//! `__crypto_edL` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The group order L as 32 little-endian byte values.
FUNC __crypto_edL() AS List OF Integer
  LET raw AS List OF Byte = encoding::hexDecode("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010")
  MUT g AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 32
    g = collections::append(g, toInt(collections::get(raw, i)))
    i = i + 1
  END WHILE
  RETURN g
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_edL", BODY));
}
