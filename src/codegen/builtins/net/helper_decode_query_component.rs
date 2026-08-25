//! `__net_decodeQueryComponent` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Query-component decoding: `%XX` plus `+` -> space. Tolerant — a malformed
' escape falls back to the raw component rather than failing the whole query
' parse (a bad query must not sink an otherwise valid request; §F.4.1 routes
' hard framing errors to 400, not soft query decode).
FUNC __net_decodeQueryComponent(s AS String) AS String
  LET decoded AS String = __net_percentDecodeImpl(s, TRUE) TRAP(e)
    RECOVER s
  END TRAP
  RETURN decoded
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_decodeQueryComponent", BODY));
}
