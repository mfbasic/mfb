//! `__audio_mmlParseUint` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse an all-digits string to a non-negative Integer, or -1 if empty or not a
' plain unsigned integer.
FUNC __audio_mmlParseUint(s AS String) AS Integer
  IF len(s) = 0 THEN
    RETURN -1
  END IF
  MUT i AS Integer = 0
  WHILE i < len(s)
    IF NOT __audio_mmlIsDigit(strings::mid(s, i, 1)) THEN
      RETURN -1
    END IF
    i = i + 1
  END WHILE
  LET v AS Integer = toInt(s) TRAP(e)
    RETURN -1
  END TRAP
  RETURN v
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlParseUint", BODY));
}
