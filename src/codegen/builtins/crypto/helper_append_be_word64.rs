//! `__crypto_appendBeWord64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Append the 64-bit word `w` to `out` as eight big-endian bytes.
FUNC __crypto_appendBeWord64(out AS List OF Byte, w AS Integer) AS List OF Byte
  MUT result AS List OF Byte = out
  MUT i AS Integer = 0
  WHILE i < 8
    LET shift AS Integer = 56 - i * 8
    LET b AS Integer = bits::band(bits::sr(w, shift), 255)
    result = collections::append(result, toByte(b))
    i = i + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_appendBeWord64", BODY));
}
