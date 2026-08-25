//! `__http_buildRequest` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_buildRequest(method AS String, url AS net::Url, body AS String, hasBody AS Boolean, headers AS Map OF String TO String) AS String
  LET crlf AS String = "\r\n"
  ' bug-262: validate every caller header (name and value) and the URL-derived
  ' request-target / Host for control bytes before any concatenation, so a
  ' `\r\n`-bearing input FAILs rather than framing a smuggled request.
  FOR EACH entry IN headers
    IF __http_hasControlBytes(entry.key) OR __http_hasControlBytes(entry.value) THEN
      FAIL error(77050002, "invalid header: control character")
    END IF
  NEXT
  IF __http_hasControlBytes(__http_requestTarget(url)) OR __http_hasControlBytes(__http_hostHeader(url)) THEN
    FAIL error(77050002, "invalid URL: control character")
  END IF
  MUT request AS String = method & " " & __http_requestTarget(url) & " HTTP/1.1" & crlf
  request = request & "Host: " & __http_headerValue(headers, "Host", __http_hostHeader(url)) & crlf
  request = request & "User-Agent: " & __http_headerValue(headers, "User-Agent", "mfb-http/1") & crlf
  request = request & "Accept: " & __http_headerValue(headers, "Accept", "*/*") & crlf
  request = request & "Connection: close" & crlf
  IF hasBody THEN
    request = request & "Content-Length: " & toString(strings::byteLen(body)) & crlf
  END IF
  FOR EACH entry IN headers
    IF __http_isExtraHeader(entry.key) THEN
      request = request & entry.key & ": " & entry.value & crlf
    END IF
  NEXT
  request = request & crlf
  IF hasBody THEN
    request = request & body
  END IF
  RETURN request
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_buildRequest", BODY));
}
