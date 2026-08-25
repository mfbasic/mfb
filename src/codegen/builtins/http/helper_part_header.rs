//! `__http_partHeader` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A part's own header value (case-insensitive) from its head block.
FUNC __http_partHeader(headStr AS String, lowerName AS String) AS String
  LET lines AS List OF String = strings::split(headStr, "\r\n")
  FOR EACH line IN lines
    LET colon AS Integer = __http_indexOf(line, ":", 0)
    IF colon >= 0 THEN
      LET nm AS String = strings::lower(strings::trim(__http_slice(line, 0, colon)))
      IF nm = lowerName THEN
        RETURN strings::trim(__http_slice(line, colon + 1, len(line)))
      END IF
    END IF
  NEXT
  RETURN ""
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_partHeader", BODY));
}
