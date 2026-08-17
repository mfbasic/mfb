//! `__regex_chr` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_chr(value AS Integer) AS String
  MUT v AS Integer = value
  IF v < 0 THEN
    v = 0
  END IF
  IF v >= 55296 AND v <= 57343 THEN
    v = 55295
  END IF
  IF v > 1114111 THEN
    v = 1114111
  END IF
  ' The clamp above guarantees a valid Unicode scalar, so utf32Decode is total
  ' here (it FAILs only on surrogate/out-of-range, both excluded by the clamp).
  RETURN encoding::utf32Decode([v])
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_chr", BODY));
}
