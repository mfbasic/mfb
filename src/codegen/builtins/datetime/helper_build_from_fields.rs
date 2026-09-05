//! `__datetime_buildFromFields` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.
//!
//! bug-519: the bound runs on the FOLDED `hour`, after the 12-hour/AM-PM
//! resolution, so `hh`+`a` is checked as the hour actually stored — and before
//! either exit builds the `Date`/`Time` record literals, which do not validate.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_buildFromFields(f AS __datetime_Fields, zone AS Zone) AS DateTime
  MUT hour AS Integer = f.hour
  IF f.is12 THEN
    IF f.isPM AND hour < 12 THEN
      hour = hour + 12
    ELSEIF f.hadPM AND NOT f.isPM AND hour = 12 THEN
      hour = 0
    END IF
  END IF
  __datetime_checkFields(f.year, f.month, f.day, hour, f.minute, f.second, f.nanos)
  LET d AS Date = Date[f.year, f.month, f.day]
  LET t AS Time = Time[hour, f.minute, f.second, f.nanos]
  IF f.hasOff THEN
    RETURN DateTime[d, t, __datetime_fixedOffset1(f.offset), f.offset]
  END IF
  RETURN __datetime_civil(d, t, zone)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_buildFromFields", BODY));
}
