//! `__crypto_gcmInc32` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Increment the low 32 bits of the 16-byte counter block `ctr` (in place copy).
FUNC __crypto_gcmInc32(ctr AS List OF Byte) AS List OF Byte
  MUT c AS List OF Byte = __crypto_copyBytes(ctr)
  MUT i AS Integer = 15
  MUT carry AS Integer = 1
  WHILE i >= 12 AND carry <> 0
    LET v AS Integer = toInt(collections::get(c, i)) + carry
    c = collections::set(c, i, toByte(bits::band(v, 255)))
    carry = bits::sr(v, 8)
    i = i - 1
  END WHILE
  RETURN c
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gcmInc32", BODY));
}
