//! `__crypto_le64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The 8-byte little-endian encoding of `value` (a length, < 2^53).
FUNC __crypto_le64(value AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT v AS Integer = value
  MUT i AS Integer = 0
  WHILE i < 8
    out = collections::append(out, toByte(bits::band(v, 255)))
    v = bits::sr(v, 8)
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_le64", BODY));
}
