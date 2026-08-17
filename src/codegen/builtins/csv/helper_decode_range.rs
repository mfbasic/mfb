//! `__csv_decodeRange` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode the half-open scalar range chars[startIndex..endIndex) directly to a
' String, reusing encoding's canonical per-code-point UTF-8 encoder.
FUNC __csv_decodeRange(chars AS List OF Integer, startIndex AS Integer, endIndex AS Integer) AS String
  MUT out AS String = ""
  MUT i AS Integer = startIndex
  WHILE i < endIndex
    LET cp AS Integer = collections::get(chars, i)
    IF cp < 0 OR cp > 1114111 THEN
      FAIL error(77050003, "invalid code point")
    END IF
    IF cp >= 55296 AND cp <= 57343 THEN
      FAIL error(77050003, "surrogate code point")
    END IF
    out = out & __encoding_fromCodepoint(cp)
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_decodeRange", BODY));
}
