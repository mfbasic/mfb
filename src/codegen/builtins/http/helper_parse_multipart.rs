//! `__http_parseMultipart` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_parseMultipart(boundary AS String, body AS List OF Byte) AS Map OF String TO RequestPart
  MUT parts AS Map OF String TO RequestPart = Map OF String TO RequestPart {}
  LET marker AS List OF Byte = strings::toBytes("--" & boundary)
  LET sep AS List OF Byte = strings::toBytes("\r\n--" & boundary)
  LET s AS Integer = __http_indexOfBytes(body, marker, 0)
  IF s < 0 THEN
    RETURN parts
  END IF
  MUT pos AS Integer = s + len(marker)
  MUT looping AS Boolean = TRUE
  WHILE looping = TRUE
    IF pos + 1 < len(body) AND collections::get(body, pos) = 45 AND collections::get(body, pos + 1) = 45 THEN
      looping = FALSE
    ELSE
      LET headStart AS Integer = pos + 2
      LET n AS Integer = __http_indexOfBytes(body, sep, headStart)
      IF n < 0 THEN
        FAIL error(errorCode::ErrInvalidFormat, "malformed multipart body")
      END IF
      parts = __http_addPart(parts, __http_byteSlice(body, headStart, n))
      pos = n + len(sep)
    END IF
  END WHILE
  RETURN parts
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_parseMultipart", BODY));
}
