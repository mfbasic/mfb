//! `__http_multipartBoundary` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_multipartBoundary(ctype AS String) AS String
  LET marker AS String = "boundary="
  LET idx AS Integer = __http_indexOf(ctype, marker, 0)
  IF idx < 0 THEN
    RETURN ""
  END IF
  MUT b AS String = __http_slice(ctype, idx + len(marker), len(ctype))
  LET semi AS Integer = __http_indexOf(b, ";", 0)
  IF semi >= 0 THEN
    b = __http_slice(b, 0, semi)
  END IF
  b = strings::trim(b)
  IF len(b) >= 2 AND strings::startsWith(b, "\"") AND strings::endsWith(b, "\"") THEN
    b = __http_slice(b, 1, len(b) - 1)
  END IF
  RETURN b
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_multipartBoundary", BODY));
}
