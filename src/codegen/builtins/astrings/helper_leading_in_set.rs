//! `__astrings_leadingInSet` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Count leading scalars of `text` that appear in the `chars` set.
FUNC __astrings_leadingInSet(text AS String, chars AS String) AS Integer
  MUT c AS Integer = 0
  FOR EACH sc IN strings::toScalars(text)
    IF strings::contains(chars, strings::fromScalars([sc])) THEN
      c = c + 1
    ELSE
      RETURN c
    END IF
  NEXT
  RETURN c
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_leadingInSet", BODY));
}
