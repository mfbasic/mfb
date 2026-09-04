//! `__http_checkResponse` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-506 / OS-55: a handler response whose reason phrase, or any header name
' or value, carries a control byte cannot be serialized without letting that
' byte split the response (a reflected `Location`, `Set-Cookie`, or CORS value
' with `\r\n` in it). Such a response is a handler defect and is answered
' exactly like a handler that raised: a built-in 500. Rejecting rather than
' stripping keeps a silently-truncated header from reaching the wire.
FUNC __http_checkResponse(resp AS Response) AS Response
  IF __http_hasFieldControlBytes(resp.reason) THEN
    RETURN __http_status(500, "Internal Server Error")
  END IF
  FOR EACH entry IN resp.headers
    IF __http_hasControlBytes(entry.key) OR __http_hasFieldControlBytes(entry.value) THEN
      RETURN __http_status(500, "Internal Server Error")
    END IF
  NEXT
  RETURN resp
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_checkResponse", BODY));
}
