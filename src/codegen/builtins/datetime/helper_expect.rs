//! `__datetime_expect` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C10: defined before __datetime_parseIso, its only caller — it was the
' one helper in this file placed after its use.
FUNC __datetime_expect(value AS String, pos AS Integer, ch AS String) AS Integer
  IF pos >= len(value) OR strings::mid(value, pos, 1) <> ch THEN
    FAIL error(77050003, "datetime: expected separator")
  END IF
  RETURN pos + 1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_expect", BODY));
}
