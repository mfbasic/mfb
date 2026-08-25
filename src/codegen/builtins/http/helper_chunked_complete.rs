//! `__http_chunkedComplete` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Whether a chunked body starting at `bodyStart` is fully present: the terminating
' zero-length chunk has been received at a real chunk boundary. Walks the chunk
' framing (mirroring `__http_dechunkBytes`) instead of substring-searching for
' `0\r\n\r\n`, which can appear INSIDE chunk data and stop the read early. Returns
' FALSE while any size line, or a chunk's data plus its trailing CRLF, is still
' incomplete, so the transport loop keeps reading; TRUE only on reaching `size = 0`.
FUNC __http_chunkedComplete(raw AS List OF Byte, bodyStart AS Integer) AS Boolean
  MUT cursor AS Integer = bodyStart
  LET total AS Integer = len(raw)
  LET crlf AS List OF Byte = strings::toBytes("\r\n")
  MUT scanning AS Boolean = TRUE
  MUT complete AS Boolean = FALSE
  WHILE scanning
    LET lineEnd AS Integer = __http_indexOfBytes(raw, crlf, cursor)
    IF lineEnd < 0 THEN
      scanning = FALSE
    ELSE
      MUT sizeText AS String = __http_bytesToText(__http_byteSlice(raw, cursor, lineEnd))
      LET semi AS Integer = __http_indexOf(sizeText, ";", 0)
      IF semi >= 0 THEN
        sizeText = __http_slice(sizeText, 0, semi)
      END IF
      LET size AS Integer = __http_hexToInt(strings::trim(sizeText))
      IF size = 0 THEN
        complete = TRUE
        scanning = FALSE
      ELSE
        LET dataEnd AS Integer = lineEnd + 2 + size
        ' need the chunk data AND its trailing CRLF before advancing
        IF dataEnd + 2 > total THEN
          scanning = FALSE
        ELSE
          cursor = dataEnd + 2
        END IF
      END IF
    END IF
  END WHILE
  RETURN complete
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_chunkedComplete", BODY));
}
