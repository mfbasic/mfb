//! `__regex_canonProp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Resolve a \p{...} property text to a canonical name, or "" if invalid.
FUNC __regex_canonProp(name AS String) AS String
  IF strings::contains(name, "=") THEN
    LET parts AS List OF String = strings::split(name, "=")
    IF len(parts) <> 2 THEN
      RETURN ""
    END IF
    LET key AS String = strings::lower(strings::trim(collections::get(parts, 0)))
    LET val AS String = strings::trim(collections::get(parts, 1))
    IF key = "gc" OR key = "general_category" THEN
      RETURN __regex_canonProp(val)
    END IF
    IF key = "sc" OR key = "script" THEN
      RETURN __regex_scriptCanon(strings::lower(val))
    END IF
    RETURN ""
  END IF
  LET low AS String = strings::lower(name)
  IF low = "l" OR low = "letter" THEN
    RETURN "L"
  END IF
  IF low = "m" OR low = "mark" THEN
    RETURN "M"
  END IF
  IF low = "n" OR low = "number" THEN
    RETURN "N"
  END IF
  IF low = "p" OR low = "punctuation" THEN
    RETURN "P"
  END IF
  IF low = "s" OR low = "symbol" THEN
    RETURN "S"
  END IF
  IF low = "z" OR low = "separator" THEN
    RETURN "Z"
  END IF
  IF low = "c" OR low = "other" THEN
    RETURN "C"
  END IF
  IF __regex_isGcName(name) THEN
    RETURN name
  END IF
  IF low = "white_space" OR low = "whitespace" THEN
    RETURN "White_Space"
  END IF
  IF low = "alphabetic" OR low = "alpha" THEN
    RETURN "Alphabetic"
  END IF
  IF __regex_isScriptName(low) THEN
    RETURN __regex_scriptCanon(low)
  END IF
  RETURN ""
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_canonProp", BODY));
}
