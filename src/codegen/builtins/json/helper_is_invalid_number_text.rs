//! `__json_isInvalidNumberText` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_isInvalidNumberText(value AS String) AS Boolean
  IF value = "nan" THEN
    RETURN TRUE
  END IF
  IF value = "-nan" THEN
    RETURN TRUE
  END IF
  IF value = "inf" THEN
    RETURN TRUE
  END IF
  IF value = "-inf" THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_isInvalidNumberText", BODY));
}
