//! `__json_trimFloatTextAt` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_trimFloatTextAt(value AS String, endIndex AS Integer) AS String
  IF endIndex <= 0 THEN
    RETURN value
  END IF
  LET last AS String = strings::mid(value, endIndex - 1, 1)
  IF last = "0" THEN
    RETURN __json_trimFloatTextAt(value, endIndex - 1)
  END IF
  IF last = "." THEN
    RETURN strings::mid(value, 0, endIndex - 1)
  END IF
  RETURN strings::mid(value, 0, endIndex)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_trimFloatTextAt", BODY));
}
