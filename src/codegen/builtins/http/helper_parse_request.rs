//! `__http_parseRequest` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_parseRequest(raw AS List OF Byte) AS Request
  IF len(raw) > __HTTP_MAX_REQUEST THEN
    FAIL error(errorCode::ErrOverflow, "request too large")
  END IF
  LET he AS Integer = __http_indexOfBytes(raw, strings::toBytes("\r\n\r\n"), 0)
  IF he < 0 THEN
    FAIL error(errorCode::ErrInvalidFormat, "incomplete request headers")
  END IF
  LET headStr AS String = __http_bytesToText(__http_byteSlice(raw, 0, he))
  IF headStr = "" THEN
    FAIL error(errorCode::ErrInvalidFormat, "non-text request headers")
  END IF
  LET lines AS List OF String = strings::split(headStr, "\r\n")
  IF len(lines) = 0 THEN
    FAIL error(errorCode::ErrInvalidFormat, "empty request")
  END IF
  LET reqLine AS String = collections::get(lines, 0)
  LET sp1 AS Integer = __http_indexOf(reqLine, " ", 0)
  IF sp1 < 0 THEN
    FAIL error(errorCode::ErrInvalidFormat, "malformed request line")
  END IF
  LET method AS String = strings::upper(__http_slice(reqLine, 0, sp1))
  LET afterMethod AS String = __http_slice(reqLine, sp1 + 1, len(reqLine))
  LET sp2 AS Integer = __http_indexOf(afterMethod, " ", 0)
  IF sp2 < 0 THEN
    FAIL error(errorCode::ErrInvalidFormat, "malformed request line")
  END IF
  LET target AS String = __http_slice(afterMethod, 0, sp2)
  MUT rawPathOnly AS String = target
  MUT queryStr AS String = ""
  LET q AS Integer = __http_indexOf(target, "?", 0)
  IF q >= 0 THEN
    rawPathOnly = __http_slice(target, 0, q)
    queryStr = __http_slice(target, q + 1, len(target))
  END IF
  MUT decodedPath AS String = rawPathOnly
  decodedPath = net::percentDecode(rawPathOnly) TRAP(e)
    RECOVER rawPathOnly
  END TRAP
  LET query AS Map OF String TO String = net::parseQuery(queryStr)
  ' bug-506 / OS-53: the strict server-side header parse and framing rules —
  ' duplicate Content-Length, Content-Length with Transfer-Encoding, whitespace
  ' before the colon, obs-fold, a non-final `chunked`, and a non-numeric
  ' Content-Length all FAIL here and become a 400. The body is exactly the
  ' framed bytes: truncated to Content-Length, or de-chunked.
  LET headers AS Map OF String TO String = __http_requestHeaderMap(headStr)
  LET framing AS Integer = __http_requestFraming(headers)
  LET bodyStart AS Integer = he + 4
  MUT body AS List OF Byte = []
  IF framing = -1 THEN
    body = __http_dechunkBytes(__http_byteSlice(raw, bodyStart, len(raw)))
  ELSE
    MUT bodyEnd AS Integer = bodyStart + framing
    IF bodyEnd > len(raw) THEN
      bodyEnd = len(raw)
    END IF
    body = __http_byteSlice(raw, bodyStart, bodyEnd)
  END IF
  MUT parts AS Map OF String TO RequestPart = Map OF String TO RequestPart {}
  LET ctype AS String = strings::lower(collections::getOr(headers, "content-type", ""))
  IF strings::contains(ctype, "multipart/form-data") THEN
    LET boundary AS String = __http_multipartBoundary(collections::getOr(headers, "content-type", ""))
    IF boundary <> "" THEN
      parts = __http_parseMultipart(boundary, body)
    END IF
  END IF
  LET emptyParams AS Map OF String TO String = Map OF String TO String {}
  RETURN Request[method, decodedPath, target, headers, query, emptyParams, parts, body]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_parseRequest", BODY));
}
