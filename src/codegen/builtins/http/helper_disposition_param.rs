//! `__http_dispositionParam` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The value of a `; name=...` / `; filename=...` parameter within a part's
' Content-Disposition. Splits on ';' so "name" never matches inside "filename".
FUNC __http_dispositionParam(disposition AS String, param AS String) AS String
  LET tokens AS List OF String = strings::split(disposition, ";")
  LET prefix AS String = param & "="
  FOR EACH tok IN tokens
    LET t AS String = strings::trim(tok)
    IF strings::startsWith(t, prefix) THEN
      MUT v AS String = __http_slice(t, len(prefix), len(t))
      IF len(v) >= 2 AND strings::startsWith(v, "\"") AND strings::endsWith(v, "\"") THEN
        v = __http_slice(v, 1, len(v) - 1)
      END IF
      RETURN v
    END IF
  NEXT
  RETURN ""
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_dispositionParam", BODY));
}
