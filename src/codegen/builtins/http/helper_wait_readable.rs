//! `__http_waitReadable` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Internal: BLOCK until the active transport is readable, bounded by the read
' deadline. The only blocking wait in the async core (`pump` stays non-blocking),
' so the blocking `read`/`write` wrappers reuse the drive loop cooperatively. The
' timed poll preserves the pre-plan-76-D read deadline (bug-268 / OS-11): the old
' blocking path set `net::setReadTimeout`, so a black-holed peer failed cleanly
' with `ErrTimeout` instead of wedging the thread. A timeout marks the stream's
' STATE `err`, which `done` treats as terminal and `finish` reports.
SUB __http_waitReadable(RES s AS Stream STATE PendingState)
  MUT rdy AS Boolean = FALSE
  MATCH s
    CASE net::Socket(p)
      rdy = net::poll(p, __HTTP_READ_TIMEOUT_MS)
    CASE tls::TlsSocket(t)
      rdy = tls::poll(t, __HTTP_READ_TIMEOUT_MS)
  END MATCH
  IF rdy = FALSE THEN
    s.state.err = errorCode::ErrTimeout
  END IF
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_waitReadable", BODY));
}
