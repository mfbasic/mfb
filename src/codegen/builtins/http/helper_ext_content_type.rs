//! `__http_extContentType` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_extContentType(path AS String) AS String
  LET lower AS String = strings::lower(path)
  LET dot AS Integer = __http_lastIndexOf(lower, ".")
  LET slash AS Integer = __http_lastIndexOf(lower, "/")
  IF dot < 0 OR dot < slash THEN
    RETURN "application/octet-stream"
  END IF
  LET ext AS String = __http_slice(lower, dot + 1, len(lower))
  IF ext = "html" OR ext = "htm" THEN RETURN "text/html; charset=utf-8"
  IF ext = "css" THEN RETURN "text/css; charset=utf-8"
  IF ext = "js" OR ext = "mjs" THEN RETURN "text/javascript; charset=utf-8"
  IF ext = "json" THEN RETURN "application/json"
  IF ext = "txt" OR ext = "text" THEN RETURN "text/plain; charset=utf-8"
  IF ext = "xml" THEN RETURN "application/xml"
  IF ext = "csv" THEN RETURN "text/csv; charset=utf-8"
  IF ext = "png" THEN RETURN "image/png"
  IF ext = "jpg" OR ext = "jpeg" THEN RETURN "image/jpeg"
  IF ext = "gif" THEN RETURN "image/gif"
  IF ext = "svg" THEN RETURN "image/svg+xml"
  IF ext = "ico" THEN RETURN "image/x-icon"
  IF ext = "webp" THEN RETURN "image/webp"
  IF ext = "woff" THEN RETURN "font/woff"
  IF ext = "woff2" THEN RETURN "font/woff2"
  IF ext = "ttf" THEN RETURN "font/ttf"
  IF ext = "pdf" THEN RETURN "application/pdf"
  IF ext = "wasm" THEN RETURN "application/wasm"
  RETURN "application/octet-stream"
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_extContentType", BODY));
}
