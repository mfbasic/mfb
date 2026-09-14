//! `__datetime_readOffset` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Reads an offset (`Z`, `±HH:MM[:SS]`, or `±HHMM[SS]`) at `start`; returns the
' offset in seconds (encoded in `.value`) and the next position.
FUNC __datetime_readOffset(value AS String, start AS Integer) AS __datetime_NumRead
  ' bug-306 S1: a zoneless timestamp leaves nothing here. `parseIso` requiring an
  ' offset is deliberate and documented (RFC 3339 always carries one), so the
  ' rejection is right -- but it must be the documented ErrInvalidFormat, not the
  ' ErrIndexOutOfRange a bare `mid` past the end would raise.
  LET head AS String = __datetime_peek(value, start, 1)
  IF head = "" THEN
    FAIL error(77050003, "datetime: expected offset")
  END IF
  IF head = "Z" OR head = "z" THEN
    RETURN __datetime_NumRead[0, start + 1]
  END IF
  MUT sign AS Integer = 1
  IF head = "-" THEN
    sign = -1
  ELSEIF head <> "+" THEN
    FAIL error(77050003, "datetime: expected offset")
  END IF
  ' bug-520 S2/S4/S5: every field is exactly two digits, minutes and seconds are
  ' 00-59 and hours 00-23 (so the magnitude is under 24 h). Anything else is
  ' malformed text -- ErrInvalidFormat, never a laundered offset such as +05:75
  ' read as +06:15, nor the zone constructor's ErrInvalidArgument. The seconds
  ' field is optional and follows the minutes in the same style: `:SS` after
  ' `HH:MM`, bare `SS` after `HHMM`.
  LET hh AS __datetime_NumRead = __datetime_readNum(value, start + 1, 2)
  IF hh.nextPos <> start + 3 OR hh.value > 23 THEN
    FAIL error(77050003, "datetime: offset hours must be two digits, 00 to 23")
  END IF
  MUT pos AS Integer = hh.nextPos
  MUT colon AS Boolean = FALSE
  IF __datetime_peek(value, pos, 1) = ":" THEN
    colon = TRUE
    pos = pos + 1
  END IF
  LET mm AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  IF mm.nextPos <> pos + 2 OR mm.value > 59 THEN
    FAIL error(77050003, "datetime: offset minutes must be two digits, 00 to 59")
  END IF
  MUT total AS Integer = hh.value * 3600 + mm.value * 60
  pos = mm.nextPos
  MUT hasSeconds AS Boolean = FALSE
  IF colon THEN
    IF __datetime_peek(value, pos, 1) = ":" THEN
      hasSeconds = TRUE
      pos = pos + 1
    END IF
  ELSEIF __datetime_isDigit(__datetime_peek(value, pos, 1)) THEN
    hasSeconds = TRUE
  END IF
  IF hasSeconds THEN
    LET ss AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
    IF ss.nextPos <> pos + 2 OR ss.value > 59 THEN
      FAIL error(77050003, "datetime: offset seconds must be two digits, 00 to 59")
    END IF
    total = total + ss.value
    pos = ss.nextPos
  END IF
  RETURN __datetime_NumRead[sign * total, pos]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_readOffset", BODY));
}
