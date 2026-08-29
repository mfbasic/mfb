//! `__crypto_x448Base` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The X448 base point u = 5 as 56 little-endian bytes (RFC 7748 §4.2).
FUNC __crypto_x448Base() AS List OF Byte
  MUT base AS List OF Byte = []
  base = collections::append(base, toByte(5))
  MUT i AS Integer = 1
  WHILE i < 56
    base = collections::append(base, toByte(0))
    i = i + 1
  END WHILE
  RETURN base
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_x448Base", BODY));
}
