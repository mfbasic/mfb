//! `__http_hasControlBytes` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-262: TRUE if `s` carries any byte below 0x20 (in particular CR 0x0D / LF
' 0x0A). A header name/value or a URL-derived request-target holding such a byte
' would, once concatenated verbatim into the raw request, let a caller embed
' `\r\n` and inject extra headers or a whole second request line — HTTP request
' splitting/smuggling against the upstream. Rejecting (not stripping) prevents a
' silently truncated header.
FUNC __http_hasControlBytes(s AS String) AS Boolean
  FOR EACH b IN strings::toBytes(s)
    IF toInt(b) < 32 THEN
      RETURN TRUE
    END IF
  NEXT
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_hasControlBytes", BODY));
}
