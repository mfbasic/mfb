//! `__net_lastIndexOf` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The index of the LAST occurrence of `needle` in `s`, or -1. RFC 3986 / WHATWG
' put the userinfo boundary at the last `@` in the authority, not the first
' (bug-306 S3).
FUNC __net_lastIndexOf(s AS String, needle AS String) AS Integer
  MUT best AS Integer = -1
  MUT at AS Integer = 0
  MUT scanning AS Boolean = TRUE
  WHILE scanning
    LET found AS Integer = __net_indexOf(s, needle, at)
    IF found < 0 THEN
      scanning = FALSE
    ELSE
      best = found
      at = found + 1
      IF at >= len(s) THEN
        scanning = FALSE
      END IF
    END IF
  END WHILE
  RETURN best
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_lastIndexOf", BODY));
}
