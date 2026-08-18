//! `__regex_propTest` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Property test against a canonical property name (see __regex_canonProp).
FUNC __regex_propTest(name AS String, cp AS Integer) AS Boolean
  LET cat AS String = __regex_genCat(cp)
  IF name = "L" THEN
    RETURN strings::startsWith(cat, "L")
  END IF
  IF name = "M" THEN
    RETURN strings::startsWith(cat, "M")
  END IF
  IF name = "N" THEN
    RETURN strings::startsWith(cat, "N")
  END IF
  IF name = "P" THEN
    RETURN strings::startsWith(cat, "P")
  END IF
  IF name = "S" THEN
    RETURN strings::startsWith(cat, "S")
  END IF
  IF name = "Z" THEN
    RETURN strings::startsWith(cat, "Z")
  END IF
  IF name = "C" THEN
    RETURN strings::startsWith(cat, "C")
  END IF
  IF name = "White_Space" THEN
    RETURN __regex_isSpaceCp(cp, cat)
  END IF
  IF name = "Alphabetic" THEN
    RETURN strings::startsWith(cat, "L") OR cat = "Nl"
  END IF
  ' POSIX class sentinels emitted by __regex_posixProp.
  IF name = "posixAlnum" THEN
    RETURN strings::startsWith(cat, "L") OR cat = "Nl" OR cat = "Nd"
  END IF
  IF name = "posixWord" THEN
    RETURN __regex_isWordCp(cp, cat)
  END IF
  IF name = "posixXdigit" THEN
    IF cp >= 48 AND cp <= 57 THEN
      RETURN TRUE
    END IF
    IF cp >= 65 AND cp <= 70 THEN
      RETURN TRUE
    END IF
    RETURN cp >= 97 AND cp <= 102
  END IF
  IF name = "posixBlank" THEN
    RETURN cp = 9 OR cat = "Zs"
  END IF
  IF name = "posixGraph" THEN
    RETURN NOT __regex_isSpaceCp(cp, cat) AND NOT strings::startsWith(cat, "C")
  END IF
  IF name = "posixPrint" THEN
    IF cat = "Zs" THEN
      RETURN TRUE
    END IF
    RETURN NOT __regex_isSpaceCp(cp, cat) AND NOT strings::startsWith(cat, "C")
  END IF
  IF __regex_isGcName(name) THEN
    RETURN cat = name
  END IF
  RETURN __regex_scriptTest(name, cp)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_propTest", BODY));
}
