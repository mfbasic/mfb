//! `__http_requestHeaderMap` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-506 / OS-53: the SERVER-side header parse. The lenient
' `__http_headerMapFromHead` stays for the client (a sloppy upstream is not a
' security boundary there); a request is parsed strictly, because every
' permissive choice here is a request-smuggling primitive against a front end
' that chose differently (RFC 9112 §5.1, §6.3):
'   - a line starting with SP/HTAB is obs-fold and is rejected, never promoted;
'   - whitespace between the field name and the colon is rejected;
'   - a line with no colon, or an empty name, is rejected;
'   - a second `Content-Length` is rejected (never last-wins);
'   - repeated `Transfer-Encoding` lines combine into one list (§5.3), so the
'     final-coding rule in `__http_requestFraming` sees the whole list.
' Other repeated fields collapse last-wins as documented. bug-507 / OS-56: the
' field count and each line's byte length are capped (`ErrMessageTooLarge`,
' answered 431).
FUNC __http_requestHeaderMap(headStr AS String) AS Map OF String TO String
  MUT headers AS Map OF String TO String = Map OF String TO String {}
  LET lines AS List OF String = strings::split(headStr, "\r\n")
  IF len(lines) - 1 > __HTTP_MAX_HEADERS THEN
    FAIL error(errorCode::ErrMessageTooLarge, "too many header fields")
  END IF
  MUT idx AS Integer = 1
  WHILE idx < len(lines)
    LET line AS String = collections::get(lines, idx)
    IF strings::byteLen(line) > __HTTP_MAX_HEADER_LINE THEN
      FAIL error(errorCode::ErrMessageTooLarge, "header line too long")
    END IF
    IF line <> "" THEN
      IF strings::startsWith(line, " ") OR strings::startsWith(line, "\t") THEN
        FAIL error(errorCode::ErrInvalidFormat, "obsolete line folding")
      END IF
      LET colon AS Integer = __http_indexOf(line, ":", 0)
      IF colon <= 0 THEN
        FAIL error(errorCode::ErrInvalidFormat, "malformed header line")
      END IF
      LET rawName AS String = __http_slice(line, 0, colon)
      IF strings::endsWith(rawName, " ") OR strings::endsWith(rawName, "\t") THEN
        FAIL error(errorCode::ErrInvalidFormat, "whitespace before header colon")
      END IF
      LET name AS String = strings::lower(rawName)
      MUT value AS String = strings::trim(__http_slice(line, colon + 1, len(line)))
      IF collections::hasKey(headers, name) THEN
        IF name = "content-length" THEN
          FAIL error(errorCode::ErrInvalidFormat, "duplicate Content-Length")
        END IF
        IF name = "transfer-encoding" THEN
          value = collections::getOr(headers, name, "") & ", " & value
        END IF
      END IF
      headers = collections::set(headers, name, value)
    END IF
    idx = idx + 1
  END WHILE
  RETURN headers
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_requestHeaderMap", BODY));
}
