//! `__datetime_readNum` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_readNum(value AS String, start AS Integer, maxDigits AS Integer) AS __datetime_NumRead
  MUT i AS Integer = start
  MUT acc AS Integer = 0
  MUT count AS Integer = 0
  LET n AS Integer = len(value)
  WHILE i < n AND count < maxDigits AND __datetime_isDigit(strings::mid(value, i, 1))
    acc = acc * 10 + toInt(strings::mid(value, i, 1))
    i = i + 1
    count = count + 1
  END WHILE
  IF count = 0 THEN
    FAIL error(77050003, "datetime: expected digits while parsing")
  END IF
  RETURN __datetime_NumRead[acc, i]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_readNum", BODY));
}
