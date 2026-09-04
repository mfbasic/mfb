//! `__http_requestFraming` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-506 / OS-53: the body length a REQUEST head implies — -1 for chunked, else
' the Content-Length, else 0 — under the strict rules (RFC 9112 §6.3): both
' Content-Length and Transfer-Encoding is rejected; `chunked` must be the exact
' FINAL transfer coding (never a substring match, never applied twice); any
' other final coding is rejected (the server cannot delimit it); Content-Length
' must be unsigned decimal digits (a bad value is a 400, never a silent 0).
FUNC __http_requestFraming(headers AS Map OF String TO String) AS Integer
  LET hasTe AS Boolean = collections::hasKey(headers, "transfer-encoding")
  LET hasCl AS Boolean = collections::hasKey(headers, "content-length")
  IF hasTe AND hasCl THEN
    FAIL error(errorCode::ErrInvalidFormat, "Content-Length with Transfer-Encoding")
  END IF
  IF hasTe THEN
    LET te AS String = strings::lower(collections::getOr(headers, "transfer-encoding", ""))
    MUT finalCoding AS String = te
    MUT earlier AS String = ""
    LET comma AS Integer = __http_lastIndexOf(te, ",")
    IF comma >= 0 THEN
      finalCoding = __http_slice(te, comma + 1, len(te))
      earlier = __http_slice(te, 0, comma)
    END IF
    IF strings::trim(finalCoding) <> "chunked" OR strings::contains(earlier, "chunked") THEN
      FAIL error(errorCode::ErrInvalidFormat, "unsupported transfer coding")
    END IF
    RETURN -1
  END IF
  IF hasCl = FALSE THEN
    RETURN 0
  END IF
  LET clText AS String = collections::getOr(headers, "content-length", "")
  IF clText = "" THEN
    FAIL error(errorCode::ErrInvalidFormat, "invalid Content-Length")
  END IF
  FOR EACH b IN strings::toBytes(clText)
    LET v AS Integer = toInt(b)
    IF v < 48 OR v > 57 THEN
      FAIL error(errorCode::ErrInvalidFormat, "invalid Content-Length")
    END IF
  NEXT
  LET cl AS Integer = toInt(clText, 10) TRAP(e)
    FAIL error(errorCode::ErrInvalidFormat, "invalid Content-Length")
  END TRAP
  RETURN cl
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_requestFraming", BODY));
}
