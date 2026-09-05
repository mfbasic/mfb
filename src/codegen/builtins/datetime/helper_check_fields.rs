//! `__datetime_checkFields` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.
//!
//! bug-519. `parse` and `parseIso` decoded their calendar fields and handed
//! them straight to record literals, which do not validate, and then to the
//! civil-days arithmetic, which is deliberately *total* (`addDays`/`addMonths`
//! need that rollover). Reached with no prior bound, totality laundered invalid
//! text into a valid-looking date: `"2026-13-45 25:70:99"` became
//! `2027-02-15T02:11:39Z` with no error, while `datetime::date(2026, 13, 45)`
//! refused the identical fields. This helper is the single place both readers
//! apply the constructors' bounds, so the package holds one position on what a
//! date is. The rollover helpers themselves are untouched.
//!
//! The code is `ErrInvalidFormat` (`77050003`), not the constructors'
//! `ErrInvalidArgument`: the *argument* is a well-formed `String`, the *text* is
//! malformed, and `77050003` is already what both readers raise for a shape
//! mismatch — so a caller wrapping them in a `TRAP` still catches one code.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"SUB __datetime_checkFields(year AS Integer, month AS Integer, day AS Integer, hour AS Integer, minute AS Integer, second AS Integer, nanos AS Integer)
  IF month < 1 OR month > 12 THEN
    FAIL error(77050003, "datetime: month out of range")
  END IF
  IF day < 1 OR day > __datetime_daysInMonth(year, month) THEN
    FAIL error(77050003, "datetime: day out of range for month")
  END IF
  IF hour < 0 OR hour > 23 THEN
    FAIL error(77050003, "datetime: hour out of range")
  END IF
  IF minute < 0 OR minute > 59 THEN
    FAIL error(77050003, "datetime: minute out of range")
  END IF
  IF second < 0 OR second > 59 THEN
    FAIL error(77050003, "datetime: second out of range")
  END IF
  IF nanos < 0 OR nanos > 999999999 THEN
    FAIL error(77050003, "datetime: nanos out of range")
  END IF
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_checkFields", BODY));
}
