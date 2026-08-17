//! `__crypto_pack25519` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_pack25519(n AS List OF Integer) AS List OF Byte
  MUT t AS List OF Integer = n
  t = __crypto_car25519(t)
  t = __crypto_car25519(t)
  t = __crypto_car25519(t)
  MUT pass2 AS Integer = 0
  WHILE pass2 < 2
    MUT m AS List OF Integer = []
    m = collections::append(m, collections::get(t, 0) - 65517)
    MUT i AS Integer = 1
    WHILE i < 15
      LET prev AS Integer = collections::get(m, i - 1)
      LET mi AS Integer = collections::get(t, i) - 65535 - bits::band(bits::sra(prev, 16), 1)
      m = collections::append(m, mi)
      m = collections::set(m, i - 1, bits::band(collections::get(m, i - 1), 65535))
      i = i + 1
    END WHILE
    LET prev14 AS Integer = collections::get(m, 14)
    LET m15 AS Integer = collections::get(t, 15) - 32767 - bits::band(bits::sra(prev14, 16), 1)
    m = collections::append(m, m15)
    LET b AS Integer = bits::band(bits::sra(m15, 16), 1)
    m = collections::set(m, 14, bits::band(collections::get(m, 14), 65535))
    IF b = 0 THEN
      t = m
    END IF
    pass2 = pass2 + 1
  END WHILE
  MUT out AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < 16
    LET ti AS Integer = collections::get(t, i)
    out = collections::append(out, toByte(bits::band(ti, 255)))
    out = collections::append(out, toByte(bits::band(bits::sr(ti, 8), 255)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pack25519", BODY));
}
