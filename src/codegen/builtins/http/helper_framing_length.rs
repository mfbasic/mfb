//! `__http_framingLength` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Body length implied by the head: Content-Length, -1 for chunked, 0 for none.
FUNC __http_framingLength(headStr AS String) AS Integer
  LET headers AS Map OF String TO String = __http_headerMapFromHead(headStr)
  LET te AS String = strings::lower(collections::getOr(headers, "transfer-encoding", ""))
  IF strings::contains(te, "chunked") THEN
    RETURN -1
  END IF
  LET clText AS String = collections::getOr(headers, "content-length", "")
  IF clText = "" THEN
    RETURN 0
  END IF
  LET cl AS Integer = toInt(strings::trim(clText), 10) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN cl
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_framingLength", BODY));
}
