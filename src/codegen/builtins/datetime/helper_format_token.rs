//! `__datetime_formatToken` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_formatToken(dt AS DateTime, ch AS String, runLen AS Integer) AS String
  IF ch = "y" THEN
    IF runLen = 2 THEN
      RETURN __datetime_pad2(__datetime_floorMod(dt.date.year, 100))
    END IF
    RETURN __datetime_padN(dt.date.year, runLen)
  END IF
  IF ch = "M" THEN
    IF runLen = 1 THEN RETURN toString(dt.date.month)
    IF runLen = 2 THEN RETURN __datetime_pad2(dt.date.month)
    IF runLen = 3 THEN RETURN __datetime_monthName(dt.date.month, FALSE)
    RETURN __datetime_monthName(dt.date.month, TRUE)
  END IF
  IF ch = "d" THEN
    IF runLen = 1 THEN RETURN toString(dt.date.day)
    RETURN __datetime_pad2(dt.date.day)
  END IF
  IF ch = "H" THEN
    IF runLen = 1 THEN RETURN toString(dt.time.hour)
    RETURN __datetime_pad2(dt.time.hour)
  END IF
  IF ch = "h" THEN
    LET h12 AS Integer = __datetime_hour12(dt.time.hour)
    IF runLen = 1 THEN RETURN toString(h12)
    RETURN __datetime_pad2(h12)
  END IF
  IF ch = "m" THEN
    IF runLen = 1 THEN RETURN toString(dt.time.minute)
    RETURN __datetime_pad2(dt.time.minute)
  END IF
  IF ch = "s" THEN
    IF runLen = 1 THEN RETURN toString(dt.time.second)
    RETURN __datetime_pad2(dt.time.second)
  END IF
  IF ch = "f" THEN
    RETURN strings::left(__datetime_padN(dt.time.nanos, 9), runLen)
  END IF
  IF ch = "a" THEN
    IF dt.time.hour < 12 THEN RETURN "AM"
    RETURN "PM"
  END IF
  IF ch = "E" THEN
    IF runLen >= 4 THEN
      RETURN __datetime_weekdayName(__datetime_isoWeekday(dt), TRUE)
    END IF
    RETURN __datetime_weekdayName(__datetime_isoWeekday(dt), FALSE)
  END IF
  IF ch = "Z" THEN
    IF runLen = 1 THEN
      IF dt.offset = 0 THEN RETURN "Z"
      RETURN __datetime_offsetLabel(dt.offset)
    END IF
    IF runLen = 2 THEN
      RETURN __datetime_offsetLabel(dt.offset)
    END IF
    RETURN __datetime_offsetLabelCompact(dt.offset)
  END IF
  FAIL error(77050003, "datetime: unknown format token")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_formatToken", BODY));
}
