//! `__crypto_reduce` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_reduce(r AS List OF Byte) AS List OF Byte
  MUT x AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 64
    x = collections::append(x, toInt(collections::get(r, i)))
    i = i + 1
  END WHILE
  RETURN __crypto_modL(x)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_reduce", BODY));
}
