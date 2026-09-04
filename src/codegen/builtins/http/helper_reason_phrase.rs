//! `__http_reasonPhrase` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_reasonPhrase(status AS Integer) AS String
  IF status = 200 THEN RETURN "OK"
  IF status = 201 THEN RETURN "Created"
  IF status = 202 THEN RETURN "Accepted"
  IF status = 204 THEN RETURN "No Content"
  IF status = 301 THEN RETURN "Moved Permanently"
  IF status = 302 THEN RETURN "Found"
  IF status = 303 THEN RETURN "See Other"
  IF status = 304 THEN RETURN "Not Modified"
  IF status = 307 THEN RETURN "Temporary Redirect"
  IF status = 308 THEN RETURN "Permanent Redirect"
  IF status = 400 THEN RETURN "Bad Request"
  IF status = 401 THEN RETURN "Unauthorized"
  IF status = 403 THEN RETURN "Forbidden"
  IF status = 404 THEN RETURN "Not Found"
  IF status = 405 THEN RETURN "Method Not Allowed"
  IF status = 408 THEN RETURN "Request Timeout"
  IF status = 409 THEN RETURN "Conflict"
  IF status = 413 THEN RETURN "Payload Too Large"
  IF status = 418 THEN RETURN "I'm a teapot"
  IF status = 422 THEN RETURN "Unprocessable Entity"
  IF status = 429 THEN RETURN "Too Many Requests"
  IF status = 431 THEN RETURN "Request Header Fields Too Large"
  IF status = 500 THEN RETURN "Internal Server Error"
  IF status = 501 THEN RETURN "Not Implemented"
  IF status = 503 THEN RETURN "Service Unavailable"
  IF status < 300 THEN RETURN "OK"
  IF status < 400 THEN RETURN "Redirect"
  IF status < 500 THEN RETURN "Client Error"
  RETURN "Server Error"
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_reasonPhrase", BODY));
}
