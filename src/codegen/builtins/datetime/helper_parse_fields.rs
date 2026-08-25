//! `__datetime_parseFields` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_parseFields(value AS String, pattern AS String) AS __datetime_Fields
  LET pn AS Integer = len(pattern)
  LET vn AS Integer = len(value)
  MUT year AS Integer = 1970
  MUT month AS Integer = 1
  MUT day AS Integer = 1
  MUT hour AS Integer = 0
  MUT minute AS Integer = 0
  MUT second AS Integer = 0
  MUT nanos AS Integer = 0
  MUT offset AS Integer = 0
  MUT hasOff AS Boolean = FALSE
  MUT isPM AS Boolean = FALSE
  MUT hadPM AS Boolean = FALSE
  MUT is12 AS Boolean = FALSE
  MUT pi AS Integer = 0
  MUT vi AS Integer = 0
  WHILE pi < pn
    LET ch AS String = strings::mid(pattern, pi, 1)
    IF ch = "'" THEN
      IF pi + 1 < pn AND strings::mid(pattern, pi + 1, 1) = "'" THEN
        IF vi >= vn OR strings::mid(value, vi, 1) <> "'" THEN
          FAIL error(77050003, "datetime: literal mismatch")
        END IF
        vi = vi + 1
        pi = pi + 2
      ELSE
        MUT pj AS Integer = pi + 1
        WHILE pj < pn AND strings::mid(pattern, pj, 1) <> "'"
          IF vi >= vn OR strings::mid(value, vi, 1) <> strings::mid(pattern, pj, 1) THEN
            FAIL error(77050003, "datetime: literal mismatch")
          END IF
          vi = vi + 1
          pj = pj + 1
        END WHILE
        pi = pj + 1
      END IF
    ELSEIF __datetime_isLetter(ch) THEN
      MUT runLen AS Integer = 1
      WHILE pi + runLen < pn AND strings::mid(pattern, pi + runLen, 1) = ch
        runLen = runLen + 1
      END WHILE
      IF ch = "y" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, runLen)
        year = r.value
        IF runLen = 2 THEN
          year = 2000 + r.value
        END IF
        vi = r.nextPos
      ELSEIF ch = "M" THEN
        IF runLen >= 3 THEN
          LET r AS __datetime_NumRead = __datetime_monthFromName(value, vi)
          month = r.value
          vi = r.nextPos
        ELSE
          LET r AS __datetime_NumRead = __datetime_readNum(value, vi, 2)
          month = r.value
          vi = r.nextPos
        END IF
      ELSEIF ch = "d" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, 2)
        day = r.value
        vi = r.nextPos
      ELSEIF ch = "H" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, 2)
        hour = r.value
        vi = r.nextPos
      ELSEIF ch = "h" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, 2)
        hour = r.value
        is12 = TRUE
        vi = r.nextPos
      ELSEIF ch = "m" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, 2)
        minute = r.value
        vi = r.nextPos
      ELSEIF ch = "s" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, 2)
        second = r.value
        vi = r.nextPos
      ELSEIF ch = "f" THEN
        LET r AS __datetime_NumRead = __datetime_readNum(value, vi, runLen)
        MUT frac AS Integer = r.value
        MUT k AS Integer = runLen
        WHILE k < 9
          frac = frac * 10
          k = k + 1
        END WHILE
        nanos = frac
        vi = r.nextPos
      ELSEIF ch = "a" THEN
        ' bug-306 S1: bounded, so a truncated AM/PM marker reports the module's
        ' documented ErrInvalidFormat rather than ErrIndexOutOfRange.
        LET marker AS String = strings::upper(__datetime_peek(value, vi, 2))
        IF marker = "PM" THEN
          isPM = TRUE
          hadPM = TRUE
        ELSEIF marker = "AM" THEN
          hadPM = TRUE
        ELSE
          FAIL error(77050003, "datetime: expected AM/PM")
        END IF
        vi = vi + 2
      ELSEIF ch = "E" THEN
        vi = __datetime_skipWeekdayName(value, vi)
      ELSEIF ch = "Z" THEN
        LET r AS __datetime_NumRead = __datetime_readOffset(value, vi)
        offset = r.value
        hasOff = TRUE
        vi = r.nextPos
      ELSE
        FAIL error(77050003, "datetime: unknown parse token")
      END IF
      pi = pi + runLen
    ELSE
      IF vi >= vn OR strings::mid(value, vi, 1) <> ch THEN
        FAIL error(77050003, "datetime: literal mismatch")
      END IF
      vi = vi + 1
      pi = pi + 1
    END IF
  END WHILE
  RETURN __datetime_Fields[year, month, day, hour, minute, second, nanos, offset, hasOff, isPM, is12, hadPM, vi]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_parseFields", BODY));
}
