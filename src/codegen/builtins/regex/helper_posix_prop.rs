//! `__regex_posixProp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Map a POSIX class name to a property usable by __regex_propTest (or a special).
FUNC __regex_posixProp(name AS String) AS String
  IF name = "alpha" THEN
    RETURN "Alphabetic"
  END IF
  IF name = "digit" THEN
    RETURN "Nd"
  END IF
  IF name = "alnum" THEN
    RETURN "posixAlnum"
  END IF
  IF name = "space" THEN
    RETURN "White_Space"
  END IF
  IF name = "upper" THEN
    RETURN "Lu"
  END IF
  IF name = "lower" THEN
    RETURN "Ll"
  END IF
  IF name = "punct" THEN
    RETURN "P"
  END IF
  IF name = "word" THEN
    RETURN "posixWord"
  END IF
  IF name = "xdigit" THEN
    RETURN "posixXdigit"
  END IF
  IF name = "blank" THEN
    RETURN "posixBlank"
  END IF
  IF name = "cntrl" THEN
    RETURN "Cc"
  END IF
  IF name = "graph" THEN
    RETURN "posixGraph"
  END IF
  IF name = "print" THEN
    RETURN "posixPrint"
  END IF
  RETURN ""
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_posixProp", BODY));
}
