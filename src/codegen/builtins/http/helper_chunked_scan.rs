//! `__http_chunkedScan` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-507 / OS-56: walk chunk framing from `cursor` (a chunk boundary) as far
' as the bytes allow. Returns -1 once the terminating zero-length chunk is
' reached, else the boundary of the first chunk that is not yet complete — the
' caller stores it and resumes there after the next read, so a body of many
' tiny chunks is walked once, not once per read. Raises `ErrInvalidFormat` on
' a malformed size line, or on a size line longer than a header line may be
' (a client that never sends the CRLF cannot make the scan unbounded).
FUNC __http_chunkedScan(raw AS List OF Byte, cursor AS Integer) AS Integer
  MUT at AS Integer = cursor
  LET total AS Integer = len(raw)
  LET crlf AS List OF Byte = strings::toBytes("\r\n")
  MUT scanning AS Boolean = TRUE
  WHILE scanning
    LET lineEnd AS Integer = __http_indexOfBytes(raw, crlf, at)
    IF lineEnd < 0 THEN
      IF total - at > __HTTP_MAX_HEADER_LINE THEN
        FAIL error(errorCode::ErrInvalidFormat, "chunk size line too long")
      END IF
      scanning = FALSE
    ELSE
      MUT sizeText AS String = __http_bytesToText(__http_byteSlice(raw, at, lineEnd))
      LET semi AS Integer = __http_indexOf(sizeText, ";", 0)
      IF semi >= 0 THEN
        sizeText = __http_slice(sizeText, 0, semi)
      END IF
      LET size AS Integer = __http_hexToInt(strings::trim(sizeText))
      IF size = 0 THEN
        RETURN -1
      END IF
      LET dataEnd AS Integer = lineEnd + 2 + size
      ' need the chunk data AND its trailing CRLF before advancing
      IF dataEnd + 2 > total THEN
        scanning = FALSE
      ELSE
        at = dataEnd + 2
      END IF
    END IF
  END WHILE
  RETURN at
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_chunkedScan", BODY));
}
