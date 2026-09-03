//! `__json_roundDigits` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-G: round an 18-digit truncation to `p` significant digits.
'
' Returns `p` digits, or `p + 1` when the carry ran off the front -- the caller
' reads the extra length as "the exponent moved by one" and keeps the first `p`.
' Returning the longer string rather than a record keeps this a pure text
' function with one result.
'
' `sticky` says whether anything non-zero followed the 18 digits. The rounding
' decision needs to know about EVERY discarded digit, so the digits between `p`
' and 18 are folded in here as well; without that a value whose tail is
' 5000...0001 would be treated as an exact tie.
'
' Half-to-EVEN at an exact tie. ECMA-262 picks the even candidate when two
' equally short renderings both read back exactly, which is why this cannot
' simply round half up.
FUNC __json_roundDigits(digits AS String, sticky AS Boolean, p AS Integer) AS String
  LET roundDigit AS String = strings::mid(digits, p, 1)
  MUT tail AS Boolean = sticky
  MUT scan AS Integer = p + 1
  WHILE scan < strings::byteLen(digits)
    IF strings::mid(digits, scan, 1) <> "0" THEN
      tail = TRUE
    END IF
    scan = scan + 1
  END WHILE
  MUT kept AS String = strings::left(digits, p)
  MUT roundUp AS Boolean = FALSE
  IF strings::contains("6789", roundDigit) THEN
    roundUp = TRUE
  ELSE
    IF roundDigit = "5" THEN
      IF tail THEN
        roundUp = TRUE
      ELSE
        ' Exact tie: up only when the last kept digit is odd.
        IF strings::contains("13579", strings::mid(kept, p - 1, 1)) THEN
          roundUp = TRUE
        END IF
      END IF
    END IF
  END IF
  IF roundUp = FALSE THEN
    RETURN kept
  END IF
  MUT index AS Integer = p - 1
  WHILE index >= 0
    LET current AS String = strings::mid(kept, index, 1)
    IF current = "9" THEN
      kept = strings::left(kept, index) & "0" & strings::mid(kept, index + 1, p - index - 1)
      index = index - 1
    ELSE
      LET bumped AS String = __json_nextDigit(current)
      RETURN strings::left(kept, index) & bumped & strings::mid(kept, index + 1, p - index - 1)
    END IF
  END WHILE
  ' Every digit was 9: the carry grows a place. The caller sees the extra
  ' length and moves the exponent, so 9.99e5 at two digits becomes 1.0e6.
  RETURN "1" & kept
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_roundDigits", BODY));
}
