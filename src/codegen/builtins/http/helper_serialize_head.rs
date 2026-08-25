//! `__http_serializeHead` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The status line + headers block. `Content-Length` and `Connection` are always
' server-supplied (a handler-set framing header is dropped, §F.5.3).
FUNC __http_serializeHead(resp AS Response) AS String
  LET crlf AS String = "\r\n"
  MUT reason AS String = resp.reason
  IF reason = "" THEN
    reason = __http_reasonPhrase(resp.status)
  END IF
  MUT head AS String = "HTTP/1.1 " & toString(resp.status) & " " & reason & crlf
  FOR EACH entry IN resp.headers
    LET lname AS String = strings::lower(entry.key)
    IF lname <> "content-length" AND lname <> "connection" THEN
      head = head & entry.key & ": " & entry.value & crlf
    END IF
  NEXT
  head = head & "Content-Length: " & toString(len(resp.body)) & crlf
  head = head & "Connection: close" & crlf
  head = head & crlf
  RETURN head
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_serializeHead", BODY));
}
