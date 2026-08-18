//! `__crypto_concat` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Concatenate two byte lists.
FUNC __crypto_concat(a AS List OF Byte, b AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = __crypto_copyBytes(a)
  LET n AS Integer = len(b)
  MUT i AS Integer = 0
  WHILE i < n
    out = collections::append(out, collections::get(b, i))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_concat", BODY));
}
