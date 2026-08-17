//! `__crypto_modL` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_modL(x AS List OF Integer) AS List OF Byte
  LET L AS List OF Integer = __crypto_edL()
  MUT xs AS List OF Integer = x
  MUT i AS Integer = 63
  WHILE i >= 32
    MUT carry AS Integer = 0
    MUT j AS Integer = i - 32
    WHILE j < i - 12
      LET xj AS Integer = collections::get(xs, j) + carry - 16 * collections::get(xs, i) * collections::get(L, j - (i - 32))
      carry = bits::sra(xj + 128, 8)
      xs = collections::set(xs, j, xj - carry * 256)
      j = j + 1
    END WHILE
    xs = collections::set(xs, j, collections::get(xs, j) + carry)
    xs = collections::set(xs, i, 0)
    i = i - 1
  END WHILE
  LET q AS Integer = bits::sra(collections::get(xs, 31), 4)
  MUT carry AS Integer = 0
  MUT j AS Integer = 0
  WHILE j < 32
    LET xj AS Integer = collections::get(xs, j) + carry - q * collections::get(L, j)
    carry = bits::sra(xj, 8)
    xs = collections::set(xs, j, bits::band(xj, 255))
    j = j + 1
  END WHILE
  j = 0
  WHILE j < 32
    xs = collections::set(xs, j, collections::get(xs, j) - carry * collections::get(L, j))
    j = j + 1
  END WHILE
  MUT out AS List OF Byte = []
  MUT k AS Integer = 0
  WHILE k < 32
    LET nextAdd AS Integer = bits::sra(collections::get(xs, k), 8)
    xs = collections::set(xs, k + 1, collections::get(xs, k + 1) + nextAdd)
    out = collections::append(out, toByte(bits::band(collections::get(xs, k), 255)))
    k = k + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_modL", BODY));
}
