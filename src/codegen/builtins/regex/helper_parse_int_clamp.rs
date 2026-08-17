//! `__regex_parseIntClamp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseIntClamp(s AS String) AS Integer
  IF len(s) > 7 THEN
    RETURN 10000000
  END IF
  RETURN toInt(s)
  TRAP(err)
    RETURN 10000000
  END TRAP
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseIntClamp", BODY));
}
