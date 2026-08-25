//! `__http_isExtraHeader` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Whether a caller header is "extra": not one of the automatic headers and not a
' reserved framing header (Content-Length / Connection are forced).
FUNC __http_isExtraHeader(name AS String) AS Boolean
  LET lowered AS String = strings::lower(name)
  IF lowered = "host" THEN
    RETURN FALSE
  ELSEIF lowered = "user-agent" THEN
    RETURN FALSE
  ELSEIF lowered = "accept" THEN
    RETURN FALSE
  ELSEIF lowered = "connection" THEN
    RETURN FALSE
  ELSEIF lowered = "content-length" THEN
    RETURN FALSE
  END IF
  RETURN TRUE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_isExtraHeader", BODY));
}
