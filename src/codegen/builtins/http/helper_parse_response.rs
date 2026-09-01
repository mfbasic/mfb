//! `__http_parseResponse` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-303: `raw` is BYTES. The head/body split is a byte-offset operation
' (CRLFCRLF is a byte sequence, and a header value may legally carry non-ASCII),
' and the body stays bytes all the way into `http::Response.body`, which is already
' `List OF Byte`. Only the head is decoded to text, and headers are ASCII by
' RFC 9110. Previously the whole response was decoded per 64 KiB receive and then
' re-encoded, which both corrupted multibyte bodies and did the work twice.
FUNC __http_parseResponse(raw AS List OF Byte) AS Response
  LET crlf AS String = "\r\n"
  LET separator AS List OF Byte = strings::toBytes(crlf & crlf)
  LET headerEnd AS Integer = __http_indexOfBytes(raw, separator, 0)
  MUT headSection AS String = ""
  MUT bodySection AS List OF Byte = []
  IF headerEnd >= 0 THEN
    headSection = __http_bytesToText(__http_byteSlice(raw, 0, headerEnd))
    bodySection = __http_byteSlice(raw, headerEnd + 4, len(raw))
  ELSE
    headSection = __http_bytesToText(raw)
  END IF

  LET lines AS List OF String = strings::split(headSection, crlf)
  IF len(lines) = 0 THEN
    FAIL error(77050003, "empty response")
  END IF
  MUT base AS Response = __http_parseStatusLine(collections::get(lines, 0))

  ' bug-339 C2: the header block (lines after the status line, lowercased,
  ' trimmed, last-wins) is exactly what __http_headerMapFromHead builds — it too
  ' skips line 0 — so parse it there instead of re-inlining the loop.
  LET headers AS Map OF String TO String = __http_headerMapFromHead(headSection)

  LET bodyBytes AS List OF Byte = __http_decodeBody(base.status, headers, bodySection)
  RETURN Response[base.status, base.reason, base.httpVersion, headers, bodyBytes, base.ok]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_parseResponse", BODY));
}
