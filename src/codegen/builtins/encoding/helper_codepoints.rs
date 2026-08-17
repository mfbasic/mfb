//! `__encoding_codepoints` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode the UTF-8 String `value` into its Unicode scalar values.
FUNC __encoding_codepoints(value AS String) AS List OF Integer
  LET data AS List OF Byte = strings::toBytes(value)
  LET n AS Integer = len(data)
  MUT result AS List OF Integer = []
  MUT i AS Integer = 0
  MUT extra AS Integer = 0
  MUT lead AS Integer = 0
  MUT codePoint AS Integer = 0
  MUT j AS Integer = 0
  MUT cont AS Integer = 0
  WHILE i < n
    lead = toInt(collections::get(data, i))
    IF lead <= 127 THEN
      result = collections::append(result, lead)
      i = i + 1
    ELSE
      IF lead >= 240 THEN
        extra = 3
        codePoint = lead - 240
      ELSE
        IF lead >= 224 THEN
          extra = 2
          codePoint = lead - 224
        ELSE
          extra = 1
          codePoint = lead - 192
        END IF
      END IF
      j = 0
      WHILE j < extra
        cont = toInt(collections::get(data, i + 1 + j))
        codePoint = codePoint * 64 + (cont - 128)
        j = j + 1
      END WHILE
      result = collections::append(result, codePoint)
      i = i + 1 + extra
    END IF
  END WHILE
  RETURN result
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_codepoints", BODY));
}
