//! `__HTTP_*` limit and deadline globals — shared private constants for the
//! `http` package.
//!
//! Registered via `add_helper`; render in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const MAX_RESPONSE: &str =
r#"LET __HTTP_MAX_RESPONSE AS Integer = 67108864"#;

#[rustfmt::skip]
const TIMEOUTS: &str =
r#"' bug-268 / OS-11: bounded default connect and per-read deadlines so a slow or
' black-holed peer (or a stalled DNS) cannot wedge the calling thread forever.
' With cooperative-only cancellation (OS-08) an unbounded blocking read would be
' uninterruptible; these deadlines make a stalled exchange fail cleanly with a
' timeout instead. 30 s comfortably exceeds a healthy request/response.
LET __HTTP_CONNECT_TIMEOUT_MS AS Integer = 30000
LET __HTTP_READ_TIMEOUT_MS AS Integer = 30000"#;

#[rustfmt::skip]
const MAX_REQUEST: &str =
r#"LET __HTTP_MAX_REQUEST AS Integer = 67108864"#;

#[rustfmt::skip]
const SERVER_LIMITS: &str =
r#"' bug-507 / OS-52, OS-56: the server-side caps and deadlines. A request head
' (request line + header block, up to and including the blank line) may not
' exceed __HTTP_MAX_HEAD bytes, carry more than __HTTP_MAX_HEADERS fields, or
' hold a line longer than __HTTP_MAX_HEADER_LINE bytes (each answered 431); a
' chunk-size line is capped at __HTTP_MAX_HEADER_LINE too (400). A connection
' that stays silent for __HTTP_SERVER_IDLE_TIMEOUT_MS between reads, or whose
' request has not completed __HTTP_SERVER_REQUEST_TIMEOUT_MS after the first
' byte, is answered 408 and closed, so one slow client (slowloris) cannot wedge
' the single-threaded accept loop.
LET __HTTP_MAX_HEAD AS Integer = 65536
LET __HTTP_MAX_HEADERS AS Integer = 100
LET __HTTP_MAX_HEADER_LINE AS Integer = 8192
LET __HTTP_SERVER_IDLE_TIMEOUT_MS AS Integer = 10000
LET __HTTP_SERVER_REQUEST_TIMEOUT_MS AS Integer = 60000"#;

/// The response-side limits render before the client helpers; the request-side
/// limit before the server helpers (matching the old `package.mfb` positions).
pub(crate) fn register_response_limits(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_maxResponse", MAX_RESPONSE));
    pkg.add_helper(RegistryHelper::always("http_timeouts", TIMEOUTS));
}

pub(crate) fn register_request_limit(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_maxRequest", MAX_REQUEST));
    pkg.add_helper(RegistryHelper::always("http_serverLimits", SERVER_LIMITS));
}
