//! `__encoding_lowBits` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Mask `value` down to its low `n` bits (n in 0..63).
FUNC __encoding_lowBits(value AS Integer, n AS Integer) AS Integer
  IF n <= 0 THEN
    RETURN 0
  END IF
  RETURN bits::band(value, bits::sl(1, n) - 1)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_lowBits", BODY));
}
