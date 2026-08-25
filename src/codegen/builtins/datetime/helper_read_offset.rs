//! `__datetime_readOffset` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Reads an offset (`Z`, `±HH:MM`, or `±HHMM`) at `start`; returns the offset in
' seconds (encoded in `.value`) and the next position.
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
  LET hh AS __datetime_NumRead = __datetime_readNum(value, start + 1, 2)
  MUT pos AS Integer = hh.nextPos
  IF pos < len(value) AND __datetime_peek(value, pos, 1) = ":" THEN
    pos = pos + 1
  END IF
  LET mm AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  RETURN __datetime_NumRead[sign * (hh.value * 3600 + mm.value * 60), mm.nextPos]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_readOffset", BODY));
}
