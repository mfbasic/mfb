//! `__datetime_padN` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-639: the width counts digits, not the sign. A negative value is its minus
' sign followed by the zero-padded digits (-1 at width 4 is -0001, never 00-1).
' The digits come from the text rather than from 0 - value, which would overflow
' for the most negative Integer.
FUNC __datetime_padN(value AS Integer, width AS Integer) AS String
  LET text AS String = toString(value)
  IF value < 0 THEN
    RETURN "-" & strings::padLeft(strings::mid(text, 1, len(text) - 1), width, "0")
  END IF
  RETURN strings::padLeft(text, width, "0")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_padN", BODY));
}
