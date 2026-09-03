//! `__json_placeDigits` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-G: ECMAScript's Number-to-String placement.
'
' Given significant digits and the exponent of the first one, decide between a
' plain decimal and an exponential form exactly as JSON.stringify does. Writing
' `n` for `exponent + 1` (the position of the point relative to the digits, the
' way ECMA-262 counts it):
'
'   1 <= n <= 21   plain, point inside the digits or zeros padded out to it
'   -6 < n <= 0    plain, "0." then -n zeros then the digits
'   otherwise      exponential, one digit before the point, `e+N` or `e-N`
'
' The boundaries are the whole point of the rule and each is exercised by the
' corpus: 1e20 is plain and 1e21 is exponential; 1e-6 is plain and 1e-7 is not.
' The exponent carries an explicit sign and is never zero-padded, so it reads
' `1e+21` and `1e-7`, not `1e+021`.
FUNC __json_placeDigits(digits AS String, exponent AS Integer, negative AS Boolean) AS String
  LET count AS Integer = strings::byteLen(digits)
  LET n AS Integer = exponent + 1
  MUT body AS String = ""
  IF n >= 1 AND n <= 21 THEN
    IF count >= n THEN
      LET head AS String = strings::left(digits, n)
      LET tail AS String = strings::mid(digits, n, count - n)
      IF tail = "" THEN
        body = head
      ELSE
        body = head & "." & tail
      END IF
    ELSE
      ' The value is a whole number wider than its digits: pad to the point.
      body = digits & strings::repeat("0", n - count)
    END IF
  ELSE
    IF n <= 0 AND n > 0 - 6 THEN
      body = "0." & strings::repeat("0", 0 - n) & digits
    ELSE
      LET head AS String = strings::left(digits, 1)
      LET tail AS String = strings::mid(digits, 1, count - 1)
      MUT mantissa AS String = head
      IF tail <> "" THEN
        mantissa = head & "." & tail
      END IF
      LET power AS Integer = n - 1
      IF power >= 0 THEN
        body = mantissa & "e+" & toString(power)
      ELSE
        body = mantissa & "e-" & toString(0 - power)
      END IF
    END IF
  END IF
  IF negative THEN
    RETURN "-" & body
  END IF
  RETURN body
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_placeDigits", BODY));
}
