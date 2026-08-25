//! `__http_headerValue` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Caller override (case-insensitive) for an automatic header, or `fallback`.
FUNC __http_headerValue(headers AS Map OF String TO String, name AS String, fallback AS String) AS String
  LET target AS String = strings::lower(name)
  FOR EACH entry IN headers
    IF strings::lower(entry.key) = target THEN
      RETURN entry.value
    END IF
  NEXT
  RETURN fallback
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_headerValue", BODY));
}
