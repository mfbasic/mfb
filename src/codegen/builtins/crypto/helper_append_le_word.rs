//! `__crypto_appendLeWord` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Append the 32-bit word `w` to `out` as four little-endian bytes.
FUNC __crypto_appendLeWord(out AS List OF Byte, w AS Integer) AS List OF Byte
  MUT result AS List OF Byte = out
  result = collections::append(result, toByte(bits::band(w, 255)))
  result = collections::append(result, toByte(bits::band(bits::sr(w, 8), 255)))
  result = collections::append(result, toByte(bits::band(bits::sr(w, 16), 255)))
  result = collections::append(result, toByte(bits::band(bits::sr(w, 24), 255)))
  RETURN result
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_appendLeWord", BODY));
}
