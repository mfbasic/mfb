//! `__http_frameComplete` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Whether `raw` holds a complete RESPONSE (headers + framed body), for the
' client's `done`. Recomputed after each pump. (The server has its own
' incremental, capped scanner — `__http_frameAdvance` — since bug-507.)
' bug-507 / OS-51: a malformed chunk size no longer raises out of the predicate;
' it reports the frame complete so the accumulated bytes go to `finish`, whose
' de-chunker raises the same `ErrInvalidFormat` as an ordinary result.
FUNC __http_frameComplete(raw AS List OF Byte) AS Boolean
  LET he AS Integer = __http_indexOfBytes(raw, strings::toBytes("\r\n\r\n"), 0)
  IF he < 0 THEN
    RETURN FALSE
  END IF
  LET headStr AS String = __http_bytesToText(__http_byteSlice(raw, 0, he))
  IF headStr = "" THEN
    RETURN TRUE
  END IF
  LET bodyStart AS Integer = he + 4
  LET have AS Integer = len(raw) - bodyStart
  LET framing AS Integer = __http_framingLength(headStr)
  IF framing = -1 THEN
    LET chunkedDone AS Boolean = __http_chunkedComplete(raw, bodyStart) TRAP(e)
      RECOVER TRUE
    END TRAP
    RETURN chunkedDone
  END IF
  RETURN have >= framing
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_frameComplete", BODY));
}
