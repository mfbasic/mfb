//! `__crypto_xorBytes` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' XOR two equal-length byte lists.
FUNC __crypto_xorBytes(a AS List OF Byte, b AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = []
  LET n AS Integer = len(a)
  MUT i AS Integer = 0
  WHILE i < n
    LET x AS Integer = toInt(collections::get(a, i))
    LET y AS Integer = toInt(collections::get(b, i))
    out = collections::append(out, toByte(bits::bxor(x, y)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_xorBytes", BODY));
}
