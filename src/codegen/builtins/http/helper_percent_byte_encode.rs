//! `__http_percentByteEncode` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-981: render one byte value as an uppercase `%XX` escape, for the three
' byte classes `__http_percentEncodeTarget` escapes (space, DEL, non-ASCII).
FUNC __http_percentByteEncode(v AS Integer) AS String
  LET hex AS String = "0123456789ABCDEF"
  RETURN "%" & strings::mid(hex, v / 16, 1) & strings::mid(hex, v - (v / 16) * 16, 1)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_percentByteEncode", BODY));
}
