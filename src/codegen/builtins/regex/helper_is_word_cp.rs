//! `__regex_isWordCp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isWordCp(cp AS Integer, cat AS String) AS Boolean
  IF __regex_catIsLetter(cat) THEN
    RETURN TRUE
  END IF
  IF cat = "Nl" THEN
    RETURN TRUE
  END IF
  IF __regex_catIsMark(cat) THEN
    RETURN TRUE
  END IF
  IF cat = "Nd" THEN
    RETURN TRUE
  END IF
  IF cat = "Pc" THEN
    RETURN TRUE
  END IF
  IF cp = 8204 OR cp = 8205 THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isWordCp", BODY));
}
