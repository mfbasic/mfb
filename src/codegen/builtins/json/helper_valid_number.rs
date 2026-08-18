//! `__json_validNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_validNumber(value AS String) AS Boolean
  LET chars AS List OF String = strings::graphemes(value)
  MUT index AS Integer = 0

  IF len(chars) = 0 THEN
    RETURN FALSE
  END IF

  IF collections::get(chars, index) = "-" THEN
    index = index + 1
    IF index >= len(chars) THEN
      RETURN FALSE
    END IF
  END IF

  IF collections::get(chars, index) = "0" THEN
    index = index + 1
  ELSE
    LET firstDigit AS String = collections::get(chars, index)
    IF __json_isNonZeroDigit(firstDigit) THEN
      index = __json_consumeDigits(chars, index + 1)
    ELSE
      RETURN FALSE
    END IF
  END IF

  IF index < len(chars) THEN
    LET decimalMark AS String = collections::get(chars, index)
    IF decimalMark = "." THEN
      index = index + 1
      IF index >= len(chars) THEN
        RETURN FALSE
      END IF
      LET fractionalDigit AS String = collections::get(chars, index)
      IF __json_isDigit(fractionalDigit) = FALSE THEN
        RETURN FALSE
      END IF
      index = __json_consumeDigits(chars, index + 1)
    END IF
  END IF

  IF index < len(chars) THEN
    LET exponentMark AS String = collections::get(chars, index)
    IF exponentMark = "e" THEN
      index = index + 1
      IF index < len(chars) THEN
        LET exponentSign AS String = collections::get(chars, index)
        IF exponentSign = "+" THEN
          index = index + 1
        ELSEIF exponentSign = "-" THEN
          index = index + 1
        END IF
      END IF
      IF index >= len(chars) THEN
        RETURN FALSE
      END IF
      LET exponentDigit AS String = collections::get(chars, index)
      IF __json_isDigit(exponentDigit) = FALSE THEN
        RETURN FALSE
      END IF
      index = __json_consumeDigits(chars, index + 1)
    ELSEIF exponentMark = "E" THEN
      index = index + 1
      IF index < len(chars) THEN
        LET exponentSign AS String = collections::get(chars, index)
        IF exponentSign = "+" THEN
          index = index + 1
        ELSEIF exponentSign = "-" THEN
          index = index + 1
        END IF
      END IF
      IF index >= len(chars) THEN
        RETURN FALSE
      END IF
      LET exponentDigit AS String = collections::get(chars, index)
      IF __json_isDigit(exponentDigit) = FALSE THEN
        RETURN FALSE
      END IF
      index = __json_consumeDigits(chars, index + 1)
    END IF
  END IF

  RETURN index = len(chars)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_validNumber", BODY));
}
