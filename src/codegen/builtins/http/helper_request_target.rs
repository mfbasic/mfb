//! `__http_requestTarget` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_requestTarget(url AS net::Url) AS String
  MUT target AS String = url.path
  IF target = "" THEN
    target = "/"
  END IF
  IF url.query <> "" THEN
    target = target & "?" & url.query
  END IF
  ' bug-981: percent-encode SP/DEL/non-ASCII so the target is always a single
  ' token on the request line (see __http_percentEncodeTarget).
  RETURN __http_percentEncodeTarget(target)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_requestTarget", BODY));
}
