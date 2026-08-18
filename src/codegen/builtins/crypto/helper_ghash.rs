//! `__crypto_ghash` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GHASH of `data` (a multiple of 16 bytes) under hash subkey H = (hHi, hLo).
FUNC __crypto_ghash(hHi AS Integer, hLo AS Integer, data AS List OF Byte) AS List OF Integer
  MUT yHi AS Integer = 0
  MUT yLo AS Integer = 0
  LET n AS Integer = len(data)
  MUT o AS Integer = 0
  WHILE o < n
    LET xHi AS Integer = __crypto_beWord64(data, o)
    LET xLo AS Integer = __crypto_beWord64(data, o + 8)
    yHi = bits::bxor(yHi, xHi)
    yLo = bits::bxor(yLo, xLo)
    LET prod AS List OF Integer = __crypto_ghashMul(yHi, yLo, hHi, hLo)
    yHi = collections::get(prod, 0)
    yLo = collections::get(prod, 1)
    o = o + 16
  END WHILE
  MUT out AS List OF Integer = []
  out = collections::append(out, yHi)
  out = collections::append(out, yLo)
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ghash", BODY));
}
