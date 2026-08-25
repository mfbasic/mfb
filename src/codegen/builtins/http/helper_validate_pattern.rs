//! `__http_validatePattern` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Validate a route pattern: `:name?` and `*` are legal only as trailing
' segment(s) (§F.3.2). Fails ErrInvalidArgument on a mid-pattern optional /
' wildcard.
SUB __http_validatePattern(pattern AS String)
  LET segs AS List OF String = __http_segments(pattern)
  LET n AS Integer = len(segs)
  MUT i AS Integer = 0
  WHILE i < n
    LET seg AS String = collections::get(segs, i)
    IF seg = "*" THEN
      IF i <> n - 1 THEN
        FAIL error(errorCode::ErrInvalidArgument, "wildcard '*' must be the final segment")
      END IF
    ELSEIF strings::endsWith(seg, "?") THEN
      MUT j AS Integer = i + 1
      WHILE j < n
        LET tail AS String = collections::get(segs, j)
        IF strings::endsWith(tail, "?") = FALSE THEN
          FAIL error(errorCode::ErrInvalidArgument, "optional ':name?' segments must be trailing")
        END IF
        j = j + 1
      END WHILE
    END IF
    i = i + 1
  END WHILE
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_validatePattern", BODY));
}
