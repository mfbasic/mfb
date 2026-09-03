//! `__json_stringifyNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-G: ECMAScript number rendering, byte-identical to JSON.stringify.
'
' The old body searched FIXED-POINT renderings -- `toString(value, places)` for
' places 1..25 -- and took the first that round-tripped. That had three
' consequences, all of them wrong against the only implementation anyone
' interoperates with:
'
'   1e21   emitted 1000000000000000000000  (Node: 1e+21)
'   1e-7   emitted a 25-digit expansion     (Node: 1e-7)
'   1e-30  FAILED with an error -- a finite number with no JSON form at all,
'          because reaching its first significant digit needs more fraction
'          places than the formatter will produce.
'
' The rewrite works in SIGNIFICANT digits instead. `json::sciParts` returns the
' first 18 significant digits of the magnitude, truncated, with the decimal
' exponent and a sticky flag saying whether anything non-zero follows:
'
'   "<sticky><18 digits>e<exponent>"      1e-7 -> "1999999999999999954e-8"
'
' From that one call the whole search runs here: round to p digits for
' p = 1..17, keep the first rendering that reads back as the same Float, then
' place the point by ECMAScript's rules. Rounding an 18-digit truncation at p
' with the sticky recomputed from the dropped digits is exactly rounding the
' exact value at p, so nothing is lost by doing it in text.
'
' Rounding is half-to-EVEN and that is load-bearing, not incidental. At an exact
' tie `toExponential` rounds half-away-from-zero and disagrees with
' JSON.stringify: 877566786661990.25 has two 16-digit forms that both read back
' exactly, and ECMA-262 says to take the even one (...990.2), which is what Node
' prints. Getting it backwards puts a fraction of a percent of all values
' silently out of step.
'
' 17 significant digits always identify a binary64, so the search always
' succeeds and there is no failure path left -- the old FAIL is gone with the
' 25-place loop that needed it.
FUNC __json_stringifyNumber(value AS Float) AS String
  __json_requireFiniteNumberText(toString(value, toByte(0)))
  IF value = 0.0 THEN
    ' Covers -0.0 as well: plan-120-C's rule that it serializes as 0, matching
    ' JSON.stringify(-0). The IEEE comparison treats the two zeros as equal.
    RETURN "0"
  END IF
  MUT negative AS Boolean = FALSE
  MUT magnitude AS Float = value
  IF value < 0.0 THEN
    negative = TRUE
    magnitude = 0.0 - value
  END IF
  LET parts AS String = json::sciParts(magnitude)
  LET sticky AS Boolean = strings::left(parts, 1) = "1"
  LET rest AS String = strings::mid(parts, 1, strings::byteLen(parts) - 1)
  LET marker AS Integer = strings::find(rest, "e")
  LET digits AS String = strings::left(rest, marker)
  LET exponent AS Integer = toInt(strings::mid(rest, marker + 1, strings::byteLen(rest) - marker - 1))
  MUT p AS Integer = 1
  WHILE p <= 17
    LET rounded AS String = __json_roundDigits(digits, sticky, p)
    ' A carry out of the leading digit lengthens the string, and the extra
    ' place belongs to the exponent: 9.99e5 at two digits is 1.0e6.
    MUT shift AS Integer = 0
    IF strings::byteLen(rounded) > p THEN
      shift = 1
    END IF
    LET kept AS String = strings::left(rounded, p)
    LET candidate AS String = __json_placeDigits(kept, exponent + shift, negative)
    IF __json_roundTrips(candidate, value) THEN
      RETURN candidate
    END IF
    p = p + 1
  END WHILE
  ' Unreachable: 17 significant digits identify every binary64, so the loop
  ' above always returns. Kept as an explicit invariant rather than a silent
  ' fall-through.
  FAIL error(77050003, "invalid JSON format")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_stringifyNumber", BODY));
}
