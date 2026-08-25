//! `__datetime_skipWeekdayName` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_skipWeekdayName(value AS String, start AS Integer) AS Integer
  MUT i AS Integer = start
  LET n AS Integer = len(value)
  WHILE i < n AND __datetime_isLetter(strings::mid(value, i, 1))
    i = i + 1
  END WHILE
  RETURN i
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_skipWeekdayName", BODY));
}
