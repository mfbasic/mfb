//! `__http_hostHeader` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_hostHeader(url AS net::Url) AS String
  IF url.port = __http_defaultPort(url.scheme) THEN
    RETURN url.host
  END IF
  RETURN url.host & ":" & toString(url.port)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_hostHeader", BODY));
}
