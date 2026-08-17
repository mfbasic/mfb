//! `__encoding_base64Symbols` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Collect 6-/5-bit symbol values from `text`, validating the alphabet and that
' '=' padding appears only as a trailing run. Returns the values; the symbol
' count is what the caller needs for length checks.
FUNC __encoding_base64Symbols(text AS String, urlSafe AS Boolean) AS List OF Integer
  LET data AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(data)
  MUT values AS List OF Integer = []
  MUT i AS Integer = 0
  MUT seenPad AS Boolean = FALSE
  MUT c AS Integer = 0
  MUT v AS Integer = 0
  WHILE i < n
    c = toInt(collections::get(data, i))
    IF c = 61 THEN
      seenPad = TRUE
    ELSE
      IF seenPad THEN
        FAIL error(77050003, "invalid base64 padding")
      END IF
      v = __encoding_base64Value(c, urlSafe)
      IF v < 0 THEN
        FAIL error(77050003, "invalid base64 character")
      END IF
      values = collections::append(values, v)
    END IF
    i = i + 1
  END WHILE
  RETURN values
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_base64Symbols", BODY));
}
