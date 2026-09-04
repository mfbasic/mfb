//! `__http_frameAdvance` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-507 / OS-56: the server's incremental request framing, run after every
' read with the bytes accumulated so far. The state carries the head-scan
' offset, the head end once found, the framing it implies, and the chunk-walk
' cursor, so each byte is examined once — the old predicate re-scanned from
' offset 0 on every read (2 MiB → 0.7 s, 64 MiB ≈ 12 min). It also enforces the
' caps: an unterminated head past __HTTP_MAX_HEAD, too many fields, or an
' over-long line is 431; a body past __HTTP_MAX_REQUEST is 413. It never
' raises: a framing defect (bug-506's strict rules, a bad chunk size) lands in
' `status` as 400 and `complete`, so the read loop stops and answers it
' (OS-51 — a raise here used to abort the process).
FUNC __http_frameAdvance(st AS __http_FrameState, raw AS List OF Byte) AS __http_FrameState
  LET total AS Integer = len(raw)
  IF total > __HTTP_MAX_REQUEST THEN
    RETURN WITH st { status := 413, complete := TRUE }
  END IF
  MUT headEnd AS Integer = st.headEnd
  MUT framing AS Integer = st.framing
  MUT cursor AS Integer = st.cursor
  IF headEnd < 0 THEN
    headEnd = __http_indexOfBytes(raw, strings::toBytes("\r\n\r\n"), st.scanFrom)
    IF headEnd < 0 THEN
      IF total > __HTTP_MAX_HEAD THEN
        RETURN WITH st { status := 431, complete := TRUE }
      END IF
      ' resume three bytes back so a terminator split across two reads is found
      MUT resume AS Integer = total - 3
      IF resume < 0 THEN
        resume = 0
      END IF
      RETURN WITH st { scanFrom := resume }
    END IF
    IF headEnd + 4 > __HTTP_MAX_HEAD THEN
      RETURN WITH st { status := 431, complete := TRUE }
    END IF
    LET headStr AS String = __http_bytesToText(__http_byteSlice(raw, 0, headEnd))
    IF headStr = "" THEN
      RETURN WITH st { status := 400, complete := TRUE }
    END IF
    MUT headErr AS Integer = 0
    framing = __http_requestFraming(__http_requestHeaderMap(headStr)) TRAP(e)
      headErr = e.code
      RECOVER 0
    END TRAP
    IF headErr = errorCode::ErrMessageTooLarge THEN
      RETURN WITH st { status := 431, complete := TRUE }
    END IF
    IF headErr <> 0 THEN
      RETURN WITH st { status := 400, complete := TRUE }
    END IF
    cursor = headEnd + 4
    IF framing > __HTTP_MAX_REQUEST - cursor THEN
      RETURN WITH st { status := 413, complete := TRUE }
    END IF
  END IF
  MUT complete AS Boolean = FALSE
  IF framing = -1 THEN
    MUT scanErr AS Integer = 0
    LET resumeAt AS Integer = __http_chunkedScan(raw, cursor) TRAP(e)
      scanErr = e.code
      RECOVER -1
    END TRAP
    IF scanErr <> 0 THEN
      RETURN WITH st { status := 400, complete := TRUE, headEnd := headEnd, framing := framing, cursor := cursor }
    END IF
    IF resumeAt = -1 THEN
      complete = TRUE
    ELSE
      cursor = resumeAt
    END IF
  ELSE
    complete = total >= cursor + framing
  END IF
  RETURN __http_FrameState[0, complete, st.scanFrom, headEnd, framing, cursor]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_frameAdvance", BODY));
}
