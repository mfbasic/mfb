//! `__http_bytesToText` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode a byte range to text, returning "" when it is not valid UTF-8.
' `toString` on a `List OF Byte` fails at runtime with ErrEncoding on invalid
' UTF-8. An inline TRAP catches that too since bug-486 made the fallibility
' census overload-aware; the function-level TRAP stays because a malicious
' non-UTF-8 header must be caught whatever the call shape, to keep the server
' crash-proof. Header blocks are ASCII, so "" reliably signals a bad request.
FUNC __http_bytesToText(b AS List OF Byte) AS String
  RETURN toString(b)
  TRAP(e)
    RETURN ""
  END TRAP
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_bytesToText", BODY));
}
