//! `__datetime_civilFromDays` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_civilFromDays(z0 AS Integer) AS Date
  LET z AS Integer = z0 + 719468
  MUT era AS Integer = 0
  IF z >= 0 THEN
    era = z / 146097
  ELSE
    era = (z - 146096) / 146097
  END IF
  LET doe AS Integer = z - era * 146097
  LET yoe AS Integer = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365
  LET y AS Integer = yoe + era * 400
  LET doy AS Integer = doe - (365 * yoe + yoe / 4 - yoe / 100)
  LET mp AS Integer = (5 * doy + 2) / 153
  LET d AS Integer = doy - (153 * mp + 2) / 5 + 1
  MUT m AS Integer = mp + 3
  IF mp >= 10 THEN
    m = mp - 9
  END IF
  MUT yFinal AS Integer = y
  IF m <= 2 THEN
    yFinal = y + 1
  END IF
  RETURN Date[yFinal, m, d]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_civilFromDays", BODY));
}
