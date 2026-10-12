//! `__net_parsePort` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse and validate an explicit port: ASCII digits only, 0..65535. Every byte
' is checked to be a digit first, which also refuses a sign (ports are unsigned,
' but toInt's signed parse would accept one). The decimal accumulator is then
' delegated to the shared radix parser, which can only fail an all-digit text by
' overflowing -- so that failure is "out of range", not "non-digit" (bug-979).
FUNC __net_parsePort(text AS String) AS Integer
  IF text = "" THEN
    FAIL error(77050003, "invalid URL: empty port")
  END IF
  LET b AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(b)
  MUT i AS Integer = 0
  WHILE i < n
    LET c AS Integer = __net_byteAt(b, i)
    IF c < 48 OR c > 57 THEN
      FAIL error(77050003, "invalid URL: non-digit in port")
    END IF
    i = i + 1
  END WHILE
  LET value AS Integer = toInt(text, 10) TRAP(err)
    FAIL error(77050003, "invalid URL: port out of range")
  END TRAP
  IF value > 65535 THEN
    FAIL error(77050003, "invalid URL: port out of range")
  END IF
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_parsePort", BODY));
}
