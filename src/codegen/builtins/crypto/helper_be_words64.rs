//! `__crypto_beWords64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Split `data` (a multiple of 8 bytes) into big-endian 64-bit words.
FUNC __crypto_beWords64(data AS List OF Byte) AS List OF Integer
  MUT out AS List OF Integer = []
  LET n AS Integer = len(data)
  MUT o AS Integer = 0
  WHILE o < n
    out = collections::append(out, __crypto_beWord64(data, o))
    o = o + 8
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_beWords64", BODY));
}
