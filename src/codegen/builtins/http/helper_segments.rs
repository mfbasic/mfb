//! `__http_segments` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_segments(path AS String) AS List OF String
  MUT out AS List OF String = []
  LET norm AS String = __http_normalizePath(path)
  IF norm = "/" OR norm = "" THEN
    RETURN out
  END IF
  MUT p AS String = norm
  IF strings::startsWith(p, "/") THEN
    p = __http_slice(p, 1, len(p))
  END IF
  RETURN strings::split(p, "/")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_segments", BODY));
}
