//! `__encoding_parseDecimal` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_parseDecimal(text AS String) AS Integer
  LET data AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(data)
  IF n = 0 THEN
    RETURN -1
  END IF
  MUT value AS Integer = 0
  MUT i AS Integer = 0
  MUT c AS Integer = 0
  WHILE i < n
    c = toInt(collections::get(data, i))
    IF c < 48 OR c > 57 THEN
      RETURN -1
    END IF
    value = value * 10 + (c - 48)
    ' bug-306 S2: stop once the value is past the maximum Unicode scalar. Checked
    ' Integer arithmetic means a long entity (`&#999...;`) would otherwise overflow
    ' i64 and fail ErrOverflow (77050010) BEFORE the caller's range check could
    ' report the module's documented ErrInvalidFormat (77050003). `-1` is the
    ' existing "not a valid entity" signal, so the caller needs no change.
    IF value > 1114111 THEN
      RETURN -1
    END IF
    i = i + 1
  END WHILE
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_parseDecimal", BODY));
}
