//! `__http_readRequestNet` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-507: read one request from a plaintext socket, bounded three ways — an
' idle deadline on every read (OS-52), a whole-request deadline, and the frame
' caps in `__http_frameAdvance` (OS-56) — and reported, never raised: the
' result's `status` is 0 for a complete request, else the 4xx to answer
' (408 idle/slow, 413/431 over a cap, 400 malformed or closed mid-request).
' A transport failure ends the read like a peer close. The TLS twin is
' `__http_readRequestTls`; the two cannot share a socket variable (§F.5.6).
FUNC __http_readRequestNet(RES sock AS tcp::Socket) AS __http_ReadResult
  tcp::setReadTimeout(sock, __HTTP_SERVER_IDLE_TIMEOUT_MS)
  LET startNs AS Integer = datetime::monotonicNanos()
  MUT raw AS List OF Byte = []
  MUT st AS __http_FrameState = __http_frameStart()
  MUT status AS Integer = 0
  MUT reading AS Boolean = TRUE
  WHILE reading
    MUT chunk AS List OF Byte = []
    MUT readErr AS Integer = 0
    chunk = tcp::read(sock, 65536) TRAP(e)
      readErr = e.code
      RECOVER []
    END TRAP
    IF readErr = errorCode::ErrTimeout THEN
      status = 408
      reading = FALSE
    ELSEIF len(chunk) = 0 THEN
      IF len(raw) > 0 THEN
        status = 400
      END IF
      reading = FALSE
    ELSE
      raw = collections::append(raw, chunk)
      st = __http_frameAdvance(st, raw)
      IF st.status <> 0 THEN
        status = st.status
        reading = FALSE
      ELSEIF st.complete THEN
        reading = FALSE
      ELSEIF datetime::monotonicNanos() - startNs > __HTTP_SERVER_REQUEST_TIMEOUT_MS * 1000000 THEN
        status = 408
        reading = FALSE
      END IF
    END IF
  END WHILE
  RETURN __http_ReadResult[raw, status]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_readRequestNet", BODY));
}
