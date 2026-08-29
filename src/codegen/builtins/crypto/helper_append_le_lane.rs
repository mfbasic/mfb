//! `__crypto_appendLeLane` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Append the 64-bit lane `lane` (a raw bit pattern) to `out` as eight little-endian bytes.
FUNC __crypto_appendLeLane(out AS List OF Byte, lane AS Integer) AS List OF Byte
  MUT result AS List OF Byte = out
  MUT i AS Integer = 0
  WHILE i < 8
    result = collections::append(result, toByte(bits::band(bits::sr(lane, i * 8), 255)))
    i = i + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_appendLeLane", BODY));
}
