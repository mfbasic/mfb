//! `__regex_shorthandMatch` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' \d \D \w \W \s \S — kind 1..6.
FUNC __regex_shorthandMatch(kind AS Integer, cp AS Integer) AS Boolean
  LET cat AS String = __regex_genCat(cp)
  IF kind = 1 THEN
    RETURN cat = "Nd"
  END IF
  IF kind = 2 THEN
    RETURN NOT (cat = "Nd")
  END IF
  IF kind = 3 THEN
    RETURN __regex_isWordCp(cp, cat)
  END IF
  IF kind = 4 THEN
    RETURN NOT __regex_isWordCp(cp, cat)
  END IF
  IF kind = 5 THEN
    RETURN __regex_isSpaceCp(cp, cat)
  END IF
  RETURN NOT __regex_isSpaceCp(cp, cat)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_shorthandMatch", BODY));
}
