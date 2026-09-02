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
' The fix searches renderings and VERIFIES the round trip rather than assuming it:
' a representation that does not parse back exactly is a formatter fault, and
' emitting silently-wrong JSON is what this bug was. The integer form is tried
' first, so a whole number stays `100` rather than `100.0`.
'
' plan-120-A corrected the claim that used to stand here, that plain
' `toString(Float)` is "the in-tree shortest-round-trip formatter". It is not:
' `_mfb_rt_float_to_string` is an exact FIXED-POINT formatter whose no-places form
' renders two decimal places (`src/codegen/string/format/float_format.rs:1-3`),
' which is why this body has to search `places` at all.
'
' plan-120-C: `-0` is emitted as `0`. The native formatter deliberately keeps
' the sign (float_format.rs: "-0.0 renders with the sign"), which is right for
' toString and stays untouched -- this maps it only on the way into JSON, where
' Node's JSON.stringify(-0) is `0` and interop is the point. The information
' loss is identical to Node's own round trip.
'
' The round-trip check below still passes after the mapping: `=` on Float lowers
' to abi::float_compare_d (an IEEE fcmp), under which +0.0 == -0.0, so
' toFloat("0") = -0.0 is TRUE and the integer branch returns "0" rather than
' falling through to the fractional search.
FUNC __json_stringifyNumber(value AS Float) AS String
  MUT integerText AS String = toString(value, toByte(0))
  __json_requireFiniteNumberText(integerText)
  IF integerText = "-0" THEN
    integerText = "0"
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
    __json_requireFiniteNumberText(candidate)
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_stringifyNumber", BODY));
}
