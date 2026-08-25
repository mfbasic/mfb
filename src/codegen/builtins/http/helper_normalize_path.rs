//! `__http_normalizePath` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_normalizePath(path AS String) AS String
  IF path = "/" THEN
    RETURN path
  END IF
  IF len(path) > 1 AND strings::endsWith(path, "/") THEN
    RETURN __http_slice(path, 0, len(path) - 1)
  END IF
  RETURN path
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_normalizePath", BODY));
}
