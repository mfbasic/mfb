//! `__net_parsePort` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse and validate an explicit port: digits only, 0..65535. The decimal
' accumulator is delegated to the shared radix parser; reject a leading sign
' first (ports are unsigned, but toInt's signed parse would accept one) and
' range-check the result afterward, keeping net's own port error messages.
FUNC __net_parsePort(text AS String) AS Integer
  IF text = "" THEN
    FAIL error(77050003, "invalid URL: empty port")
  END IF
  IF strings::startsWith(text, "-") OR strings::startsWith(text, "+") THEN
    FAIL error(77050003, "invalid URL: non-digit in port")
  END IF
  LET value AS Integer = toInt(text, 10) TRAP(err)
    FAIL error(77050003, "invalid URL: non-digit in port")
  END TRAP
  IF value > 65535 THEN
    FAIL error(77050003, "invalid URL: port out of range")
  END IF
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_parsePort", BODY));
}
