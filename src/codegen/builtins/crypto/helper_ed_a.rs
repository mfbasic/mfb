//! `__crypto_edA` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_edA(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    o = collections::append(o, collections::get(a, i) + collections::get(b, i))
    i = i + 1
  END WHILE
  RETURN o
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_edA", BODY));
}
