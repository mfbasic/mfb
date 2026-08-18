//! `__crypto_unpack25519` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_unpack25519(b AS List OF Byte) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    LET lo AS Integer = toInt(collections::get(b, 2 * i))
    LET hi AS Integer = toInt(collections::get(b, 2 * i + 1))
    o = collections::append(o, lo + bits::sl(hi, 8))
    i = i + 1
  END WHILE
  o = collections::set(o, 15, bits::band(collections::get(o, 15), 32767))
  RETURN o
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_unpack25519", BODY));
}
