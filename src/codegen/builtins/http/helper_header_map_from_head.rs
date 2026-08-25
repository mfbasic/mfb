//! `__http_headerMapFromHead` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse the header block (excluding the request line at index 0) into a map
' with lowercased field names; duplicates collapse last-wins (§F.4.1).
FUNC __http_headerMapFromHead(headStr AS String) AS Map OF String TO String
  MUT headers AS Map OF String TO String = Map OF String TO String {}
  LET lines AS List OF String = strings::split(headStr, "\r\n")
  MUT idx AS Integer = 1
  WHILE idx < len(lines)
    LET line AS String = collections::get(lines, idx)
    IF line <> "" THEN
      LET colon AS Integer = __http_indexOf(line, ":", 0)
      IF colon >= 0 THEN
        LET name AS String = strings::lower(strings::trim(__http_slice(line, 0, colon)))
        LET value AS String = strings::trim(__http_slice(line, colon + 1, len(line)))
        headers = collections::set(headers, name, value)
      END IF
    END IF
    idx = idx + 1
  END WHILE
  RETURN headers
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_headerMapFromHead", BODY));
}
