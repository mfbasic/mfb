//! `__crypto_concatInt` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_concatInt(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  MUT o AS List OF Integer = a
  LET n AS Integer = len(b)
  MUT i AS Integer = 0
  WHILE i < n
    o = collections::append(o, collections::get(b, i))
    i = i + 1
  END WHILE
  RETURN o
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_concatInt", BODY));
}
