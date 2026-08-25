//! `__astrings_isWinner` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Whether `s` is the winning span for its (class, member) among `covering`:
REM no other covering span of the same class+member has a higher start, or an
REM equal start with a higher seq (later insertion). Exactly one span per
REM (class, member) wins, since seq is unique.
FUNC __astrings_isWinner(s AS AttrSpan, covering AS List OF AttrSpan) AS Boolean
  FOR EACH t IN covering
    IF t.class = s.class AND t.member = s.member THEN
      IF t.start > s.start THEN
        RETURN FALSE
      END IF
      IF t.start = s.start AND t.seq > s.seq THEN
        RETURN FALSE
      END IF
    END IF
  NEXT
  RETURN TRUE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_isWinner", BODY));
}
