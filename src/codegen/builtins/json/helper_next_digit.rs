//! `__json_nextDigit` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-G: the successor of a decimal digit, as text.
'
' `__json_roundDigits` needs to add one to a single digit it is holding as a
' string. Going through `toInt`/`toString` would work, but a nine-arm lookup
' cannot fail and cannot be affected by anything else, which suits a helper on
' the rounding path. `9` never reaches here -- the caller handles the carry.
FUNC __json_nextDigit(digit AS String) AS String
  IF digit = "0" THEN
    RETURN "1"
  END IF
  IF digit = "1" THEN
    RETURN "2"
  END IF
  IF digit = "2" THEN
    RETURN "3"
  END IF
  IF digit = "3" THEN
    RETURN "4"
  END IF
  IF digit = "4" THEN
    RETURN "5"
  END IF
  IF digit = "5" THEN
    RETURN "6"
  END IF
  IF digit = "6" THEN
    RETURN "7"
  END IF
  IF digit = "7" THEN
    RETURN "8"
  END IF
  RETURN "9"
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_nextDigit", BODY));
}
