//! `__astrings_mdEscapeFont` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Escape a font name: the text delimiters plus `;` (the font/size separator).
REM A font whose escaped form is a bare `-` would read as a reset, so a literal
REM `-` is escaped too.
FUNC __astrings_mdEscapeFont(s AS String) AS String
  IF s = "-" THEN
    RETURN "\\-"
  END IF
  MUT out AS String = ""
  FOR EACH sc IN strings::toScalars(s)
    LET ch AS String = strings::fromScalars([sc])
    IF ch = "\\" OR ch = "*" OR ch = "_" OR ch = "~" OR ch = "^" OR ch = ":" OR ch = ";" THEN
      out = out & "\\" & ch
    ELSE
      out = out & ch
    END IF
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_mdEscapeFont", BODY));
}
