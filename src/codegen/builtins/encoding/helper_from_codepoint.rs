//! `__encoding_fromCodepoint` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' UTF-8 encode a single Unicode scalar value to its String form.
FUNC __encoding_fromCodepoint(value AS Integer) AS String
  IF value < 0 THEN
    FAIL error(77050003, "invalid code point")
  END IF
  IF value <= 127 THEN
    LET one AS List OF Byte = [toByte(value)]
    RETURN toString(one)
  END IF
  IF value <= 2047 THEN
    LET lead AS Byte = toByte(192 + value / 64)
    LET tailValue AS Integer = value - (value / 64) * 64
    LET tail AS Byte = toByte(128 + tailValue)
    LET two AS List OF Byte = [lead, tail]
    RETURN toString(two)
  END IF
  IF value <= 65535 THEN
    LET lead AS Byte = toByte(224 + value / 4096)
    LET rem1 AS Integer = value - (value / 4096) * 4096
    LET midValue AS Integer = rem1 / 64
    LET tailValue AS Integer = rem1 - midValue * 64
    LET middle AS Byte = toByte(128 + midValue)
    LET tail AS Byte = toByte(128 + tailValue)
    LET three AS List OF Byte = [lead, middle, tail]
    RETURN toString(three)
  END IF
  IF value <= 1114111 THEN
    LET lead AS Byte = toByte(240 + value / 262144)
    LET rem1 AS Integer = value - (value / 262144) * 262144
    LET nextValue AS Integer = rem1 / 4096
    LET rem2 AS Integer = rem1 - nextValue * 4096
    LET midValue AS Integer = rem2 / 64
    LET tailValue AS Integer = rem2 - midValue * 64
    LET second AS Byte = toByte(128 + nextValue)
    LET third AS Byte = toByte(128 + midValue)
    LET tail AS Byte = toByte(128 + tailValue)
    LET four AS List OF Byte = [lead, second, third, tail]
    RETURN toString(four)
  END IF
  FAIL error(77050003, "invalid code point")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_fromCodepoint", BODY));
}
