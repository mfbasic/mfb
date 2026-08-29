//! `__crypto_padLimbs` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' `x` extended with zero limbs to at least `n` limbs (a length-only decision).
FUNC __crypto_padLimbs(x AS List OF Integer, n AS Integer) AS List OF Integer
  MUT o AS List OF Integer = x
  WHILE len(o) < n
    o = collections::append(o, 0)
  END WHILE
  RETURN o
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_padLimbs", BODY));
}
