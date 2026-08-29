//! `__crypto_bytesToLimbs` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A little-endian byte string as byte limbs (`List OF Integer`, each 0..255).
FUNC __crypto_bytesToLimbs(b AS List OF Byte) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < len(b)
    o = collections::append(o, toInt(collections::get(b, i)))
    i = i + 1
  END WHILE
  RETURN o
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_bytesToLimbs", BODY));
}
