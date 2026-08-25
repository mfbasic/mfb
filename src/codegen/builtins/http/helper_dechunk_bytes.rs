//! `__http_dechunkBytes` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' De-chunk a Transfer-Encoding: chunked request body (byte-accurate).
FUNC __http_dechunkBytes(raw AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT cursor AS Integer = 0
  LET total AS Integer = len(raw)
  MUT done AS Boolean = FALSE
  LET crlf AS List OF Byte = strings::toBytes("\r\n")
  WHILE done = FALSE
    LET lineEnd AS Integer = __http_indexOfBytes(raw, crlf, cursor)
    IF lineEnd < 0 THEN
      FAIL error(errorCode::ErrInvalidFormat, "malformed chunk framing")
    END IF
    MUT sizeText AS String = __http_bytesToText(__http_byteSlice(raw, cursor, lineEnd))
    LET semi AS Integer = __http_indexOf(sizeText, ";", 0)
    IF semi >= 0 THEN
      sizeText = __http_slice(sizeText, 0, semi)
    END IF
    LET size AS Integer = __http_hexToInt(strings::trim(sizeText))
    LET dataStart AS Integer = lineEnd + 2
    IF size = 0 THEN
      done = TRUE
    ELSE
      LET dataEnd AS Integer = dataStart + size
      IF dataEnd > total THEN
        FAIL error(errorCode::ErrInvalidFormat, "truncated chunk data")
      END IF
      out = collections::append(out, __http_byteSlice(raw, dataStart, dataEnd))
      cursor = dataEnd + 2
    END IF
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_dechunkBytes", BODY));
}
