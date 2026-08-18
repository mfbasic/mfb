//! `__crypto_beWords` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Split `data` (a multiple of 4 bytes) into big-endian 32-bit words.
FUNC __crypto_beWords(data AS List OF Byte) AS List OF Integer
  MUT out AS List OF Integer = []
  LET n AS Integer = len(data)
  MUT o AS Integer = 0
  WHILE o < n
    LET w AS Integer = __crypto_beWord(data, o)
    out = collections::append(out, w)
    o = o + 4
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_beWords", BODY));
}
