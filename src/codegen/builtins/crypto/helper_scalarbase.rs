//! `__crypto_scalarbase` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_scalarbase(s AS List OF Byte) AS List OF Integer
  LET x AS List OF Integer = __crypto_gfX()
  LET y AS List OF Integer = __crypto_gfY()
  LET z AS List OF Integer = __crypto_gf1()
  LET t AS List OF Integer = __crypto_edM(x, y)
  LET base AS List OF Integer = __crypto_point4(x, y, z, t)
  RETURN __crypto_scalarmult(base, s)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_scalarbase", BODY));
}
