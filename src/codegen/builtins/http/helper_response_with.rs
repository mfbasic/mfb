//! `__http_responseWith` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_responseWith(status AS Integer, body AS String, contentType AS String) AS Response
  MUT h AS Map OF String TO String = Map OF String TO String {}
  IF contentType <> "" THEN
    h = collections::set(h, "content-type", contentType)
  END IF
  LET ok AS Boolean = status >= 200 AND status <= 299
  RETURN Response[status, "", "1.1", h, strings::toBytes(body), ok]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_responseWith", BODY));
}
