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

/// The response-side limits render before the client helpers; the request-side
/// limit before the server helpers (matching the old `package.mfb` positions).
pub(crate) fn register_response_limits(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_maxResponse", MAX_RESPONSE));
    pkg.add_helper(RegistryHelper::always("http_timeouts", TIMEOUTS));
}

pub(crate) fn register_request_limit(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_maxRequest", MAX_REQUEST));
}
