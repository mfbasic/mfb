//! `__datetime_monthFromName` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_monthFromName(value AS String, start AS Integer) AS __datetime_NumRead
  LET lowered AS String = strings::lower(value)
  MUT m AS Integer = 1
  WHILE m <= 12
    LET full AS String = strings::lower(__datetime_monthName(m, TRUE))
    LET flen AS Integer = len(full)
    LET candFull AS String = __datetime_peek(lowered, start, flen)
    IF candFull = full THEN
      RETURN __datetime_NumRead[m, start + flen]
    END IF
    LET short AS String = strings::lower(__datetime_monthName(m, FALSE))
    LET candShort AS String = __datetime_peek(lowered, start, 3)
    IF candShort = short THEN
      RETURN __datetime_NumRead[m, start + 3]
    END IF
    m = m + 1
  END WHILE
  FAIL error(77050003, "datetime: unrecognized month name")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_monthFromName", BODY));
}
