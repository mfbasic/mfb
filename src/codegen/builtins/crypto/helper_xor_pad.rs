//! `__crypto_xorPad` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' XOR every byte of `data` with the constant `pad` (0x36 or 0x5c).
FUNC __crypto_xorPad(data AS List OF Byte, pad AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  LET n AS Integer = len(data)
  MUT i AS Integer = 0
  WHILE i < n
    LET b AS Integer = toInt(collections::get(data, i))
    out = collections::append(out, toByte(bits::bxor(b, pad)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_xorPad", BODY));
}
