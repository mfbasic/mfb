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
  ' bug-858: a response whose status makes it categorically bodyless (RFC 9112
  ' §6.3 rules 1-2: 1xx, 204, 304) is complete at the end of the head, before
  ' ANY Content-Length/Transfer-Encoding/close-delimited arm below -- those
  ' headers describe a body this status guarantees was never sent. This
  ' mirrors `__http_decodeBody`, which already discards the body for 204/304
  ' on the same basis. `__http_statusOfHead` never raises (see its own
  ' comment), so a status line this helper cannot parse falls through to the
  ' ordinary framing checks unresolved, and a genuinely malformed status line
  ' still surfaces from `finish`'s parse, not from `done`'s completion check --
  ' `done`'s declared Errors table is unaffected by this change.
  LET status AS Integer = __http_statusOfHead(headStr)
  IF status >= 100 AND status <= 199 THEN
    RETURN TRUE
  END IF
  IF status = 204 OR status = 304 THEN
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
  ' bug-858: no Content-Length, no chunked — close-delimited (RFC 9112 §6.3
  ' rule 8). There is no frame to satisfy; the only exit is the peer's EOF,
  ' which `__http_done` already checks via `state.closed`.
  IF framing = -2 THEN
    RETURN FALSE
  END IF
  RETURN have >= framing
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_frameComplete", BODY));
}
