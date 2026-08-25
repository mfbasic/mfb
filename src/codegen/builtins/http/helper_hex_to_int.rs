//! `__http_hexToInt` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse an unsigned chunk-size hex field via the shared radix parser. The
' digit-by-digit accumulator is gone; toInt(text, 16) owns the conversion and
' overflow check, while this wrapper keeps http's own empty/invalid messages
' (toInt's signed parse would otherwise accept a leading sign, so reject one
' first — chunk sizes are unsigned).
FUNC __http_hexToInt(text AS String) AS Integer
  IF text = "" THEN
    FAIL error(77050003, "empty chunk size")
  END IF
  IF strings::startsWith(text, "-") OR strings::startsWith(text, "+") THEN
    FAIL error(77050003, "invalid chunk size")
  END IF
  LET value AS Integer = toInt(text, 16) TRAP(err)
    FAIL error(77050003, "invalid chunk size")
  END TRAP
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_hexToInt", BODY));
}
