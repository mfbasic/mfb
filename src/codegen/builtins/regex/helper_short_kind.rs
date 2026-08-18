//! `__regex_shortKind` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_shortKind(e AS String) AS Integer
  IF e = "d" THEN
    RETURN 1
  END IF
  IF e = "D" THEN
    RETURN 2
  END IF
  IF e = "w" THEN
    RETURN 3
  END IF
  IF e = "W" THEN
    RETURN 4
  END IF
  IF e = "s" THEN
    RETURN 5
  END IF
  IF e = "S" THEN
    RETURN 6
  END IF
  RETURN 0
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_shortKind", BODY));
}
