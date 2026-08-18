//! `__crypto_appendBeWord` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Append the 32-bit word `w` to `out` as four big-endian bytes.
FUNC __crypto_appendBeWord(out AS List OF Byte, w AS Integer) AS List OF Byte
  MUT result AS List OF Byte = out
  LET a AS Integer = bits::band(bits::sr(w, 24), 255)
  LET b AS Integer = bits::band(bits::sr(w, 16), 255)
  LET c AS Integer = bits::band(bits::sr(w, 8), 255)
  LET d AS Integer = bits::band(w, 255)
  result = collections::append(result, toByte(a))
  result = collections::append(result, toByte(b))
  result = collections::append(result, toByte(c))
  result = collections::append(result, toByte(d))
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_appendBeWord", BODY));
}
