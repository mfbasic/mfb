//! `__astrings_mdEscape` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Backslash-escape the delimiter characters in visible text.
FUNC __astrings_mdEscape(s AS String) AS String
  MUT out AS String = ""
  FOR EACH sc IN strings::toScalars(s)
    LET ch AS String = strings::fromScalars([sc])
    IF ch = "\\" OR ch = "*" OR ch = "_" OR ch = "~" OR ch = "^" OR ch = ":" THEN
      out = out & "\\" & ch
    ELSE
      out = out & ch
    END IF
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_mdEscape", BODY));
}
