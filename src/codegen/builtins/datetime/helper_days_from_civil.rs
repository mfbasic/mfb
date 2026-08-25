//! `__datetime_daysFromCivil` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_daysFromCivil(y0 AS Integer, m AS Integer, d AS Integer) AS Integer
  MUT y AS Integer = y0
  IF m <= 2 THEN
    y = y - 1
  END IF
  MUT era AS Integer = 0
  IF y >= 0 THEN
    era = y / 400
  ELSE
    era = (y - 399) / 400
  END IF
  LET yoe AS Integer = y - era * 400
  MUT mAdj AS Integer = 9
  IF m > 2 THEN
    mAdj = -3
  END IF
  LET doy AS Integer = (153 * (m + mAdj) + 2) / 5 + d - 1
  LET doe AS Integer = yoe * 365 + yoe / 4 - yoe / 100 + doy
  RETURN era * 146097 + doe - 719468
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_daysFromCivil", BODY));
}
