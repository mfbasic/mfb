//! `__json_arrayIndex` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-B: the array-index grammar for a `json::get`/`json::getOr` path step,
' taken from RFC 6901 (JSON Pointer): a single `0`, or a nonzero digit followed by
' further digits. No sign, no leading zero, no surrounding space -- so "01", "+1"
' and "1 " are NOT indexes and read as an ordinary miss. Returns -1 for anything
' that is not an index, which both callers already treat as "cannot continue".
'
' Tokens longer than 18 digits return -1 rather than being converted. 18 digits
' is the widest value that cannot overflow a 64-bit Integer, and no List can hold
' 10^18 items, so such a token is out of range no matter what it converts to --
' the outcome is the same miss, and `toInt`'s ErrOverflow never escapes. That
' matters most for `json::getOr`, whose whole contract is that it does not fail.
'
' The digit test is a SUBSTRING search, not the `ch >= "0" AND ch <= "9"` range
' compare the parser's `__json_isDigit` used to make (the parser now compares
' bytes). That compare was right for the parser, which fed it one scanned
' character at a time, but wrong here: a path token is arbitrary
' caller text, and `strings::graphemes` yields whole grapheme clusters. The
' cluster "1" + U+0308 COMBINING DIAERESIS sorts INSIDE ["0", "9"] because
' comparison is lexicographic and it starts with "1" -- so the range compare
' called it a digit, `toInt("1<U+0308>")` then raised ErrInvalidFormat, and that
' escaped `json::getOr`. Measured before the fix:
'   [1 + combining-1] getOr: FAILED 77050003
' A substring test cannot make that mistake: the cluster is not a substring of
' "0123456789". This mirrors `__json_isNonZeroDigit`, which is why the FIRST
' character was never affected.
FUNC __json_arrayIndex(token AS String) AS Integer
  LET chars AS List OF String = strings::graphemes(token)
  LET n AS Integer = len(chars)
  IF n = 0 OR n > 18 THEN
    RETURN 0 - 1
  END IF
  LET first AS String = collections::get(chars, 0)
  IF n = 1 AND first = "0" THEN
    RETURN 0
  END IF
  IF __json_isNonZeroDigit(first) = FALSE THEN
    RETURN 0 - 1
  END IF
  MUT i AS Integer = 1
  WHILE i < n
    IF strings::contains("0123456789", collections::get(chars, i)) = FALSE THEN
      RETURN 0 - 1
    END IF
    i = i + 1
  END WHILE
  RETURN toInt(token)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_arrayIndex", BODY));
}
