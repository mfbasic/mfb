//! `__json_trimFloatText` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_trimFloatText(value AS String) AS String
  IF strings::contains(value, ".") = FALSE THEN
    RETURN value
  END IF
  RETURN __json_trimFloatTextAt(value, len(value))
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_trimFloatText", BODY));
}
