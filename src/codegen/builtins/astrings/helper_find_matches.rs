//! `__astrings_findMatches` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Non-overlapping left-to-right scalar start indices of `needle` in `text`.
FUNC __astrings_findMatches(text AS String, needle AS String) AS List OF Integer
  MUT matches AS List OF Integer = []
  LET nlen AS Integer = __astrings_scalarCountStr(needle)
  IF nlen = 0 THEN
    RETURN matches
  END IF
  LET tlen AS Integer = __astrings_scalarCountStr(text)
  MUT pos AS Integer = 0
  WHILE pos <= tlen - nlen
    IF strings::mid(text, pos, nlen) = needle THEN
      matches = collections::append(matches, pos)
      pos = pos + nlen
    ELSE
      pos = pos + 1
    END IF
  END WHILE
  RETURN matches
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_findMatches", BODY));
}
