//! `__http_normalizeMethod` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-506 / OS-54: the method is the first token on the wire, so a control byte
' in it (CR, LF, NUL, ...) frames extra header lines or a whole second request
' line — bug-262's sweep covered the headers and the target but not the method.
FUNC __http_normalizeMethod(method AS String) AS String
  IF method = "" THEN
    FAIL error(77050002, "empty HTTP method")
  END IF
  IF strings::contains(method, " ") OR __http_hasControlBytes(method) THEN
    FAIL error(77050002, "invalid HTTP method")
  END IF
  RETURN strings::upper(method)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_normalizeMethod", BODY));
}
