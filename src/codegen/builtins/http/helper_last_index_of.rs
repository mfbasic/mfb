//! `__http_lastIndexOf` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Grapheme index of the last occurrence of `needle` in `s`, or -1.
FUNC __http_lastIndexOf(s AS String, needle AS String) AS Integer
  MUT idx AS Integer = -1
  MUT from AS Integer = 0
  MUT scanning AS Boolean = TRUE
  WHILE scanning = TRUE
    LET found AS Integer = __http_indexOf(s, needle, from)
    IF found < 0 THEN
      scanning = FALSE
    ELSE
      idx = found
      from = found + 1
    END IF
  END WHILE
  RETURN idx
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_lastIndexOf", BODY));
}
