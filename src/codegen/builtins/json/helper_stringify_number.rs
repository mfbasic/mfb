//! `__json_stringifyNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-304: the fractional path used a fixed `toString(value, toByte(9))`. Nine
' decimal places cannot represent a general binary64, so significant digits were
' silently dropped and `json::parse` after `json::stringify` was NOT the identity
' on numbers -- 3.141592653589793 came back as 3.141592654. Only the integer path
' was round-trip-checked.
'
' The plain `toString(Float)` is the in-tree shortest-round-trip formatter
' (`_mfb_rt_float_to_string`), so it already emits the shortest decimal that parses
' back to the same Float. The integer form is still tried first, so a whole number
' stays `100` rather than `100.0`. The round-trip is then VERIFIED rather than
' assumed: a representation that does not parse back exactly is a formatter fault,
' and emitting silently-wrong JSON is what this bug was.
FUNC __json_stringifyNumber(value AS Float) AS String
  LET integerText AS String = toString(value, toByte(0))
  IF __json_isInvalidNumberText(integerText) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  IF toFloat(integerText) = value THEN
    RETURN integerText
  END IF
  ' Search for the SHORTEST precision that parses back to the same Float. 17
  ' significant digits always suffice for binary64, and `toString(Float)` counts
  ' digits after the point, so 17 fractional digits covers every value whose
  ' integer part is non-zero; smaller magnitudes need more, hence the 25 bound.
  MUT places AS Integer = 1
  WHILE places <= 25
    LET candidate AS String = toString(value, toByte(places))
    IF __json_isInvalidNumberText(candidate) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET shortened AS String = __json_trimFloatText(candidate)
    IF toFloat(shortened) = value THEN
      RETURN shortened
    END IF
    places = places + 1
  END WHILE
  ' No representable rendering round-trips; emitting a silently-lossy number is
  ' exactly the defect this fixes, so fail loudly instead.
  FAIL error(77050003, "invalid JSON format")
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_stringifyNumber", BODY));
}
