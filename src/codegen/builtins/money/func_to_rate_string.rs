//! `money::toRate(String)` — parse decimal text exactly into a `Rate`.
//!
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::RATE_TYPE;
use crate::codegen::registry::{Body, DefaultValue, Implementation, Parameter};
use crate::types::ParameterType;

/// One pass over the bytes, integer arithmetic only, so the text is read exactly.
///
/// The grammar is an optional `-`, at least one digit, then optionally a `.` and
/// at least one more digit. Anything else, or more than 12 fraction digits, is
/// `ErrInvalidFormat`. The errors are then checked in a fixed order: the format
/// first, then a value at or below zero (`ErrInvalidArgument`, so `-1` is never an
/// overflow), then a value too large to scale (`ErrOverflow`).
///
/// The whole part stops accumulating once it passes `9223372` (the largest whole
/// part that can fit), so a long run of digits cannot overflow the accumulator;
/// `wide` remembers that it did. With the whole part at most `9223372`, its scaled
/// value is at most `9223372000000000000`, and only the fraction can push the sum
/// past the largest `Integer` — which is what the last comparison tests.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __money_toRateString(text AS String) AS Rate
  MUT negative AS Boolean = FALSE
  MUT point AS Boolean = FALSE
  MUT first AS Boolean = TRUE
  MUT nonzero AS Boolean = FALSE
  MUT wide AS Boolean = FALSE
  MUT wholeDigits AS Integer = 0
  MUT fractionDigits AS Integer = 0
  MUT whole AS Integer = 0
  MUT fraction AS Integer = 0
  MUT d AS Integer = 0
  FOR EACH b IN strings::toBytes(text)
    d = toInt(b)
    IF first AND d = 45 THEN
      negative = TRUE
    ELSEIF d = 46 AND NOT point AND wholeDigits > 0 THEN
      point = TRUE
    ELSEIF d >= 48 AND d <= 57 THEN
      d = d - 48
      IF d <> 0 THEN
        nonzero = TRUE
      END IF
      IF point THEN
        fractionDigits = fractionDigits + 1
        IF fractionDigits > 12 THEN
          FAIL error(77050003, "a rate has at most 12 decimal places: " & text)
        END IF
        fraction = fraction * 10 + d
      ELSE
        wholeDigits = wholeDigits + 1
        IF NOT wide THEN
          whole = whole * 10 + d
          IF whole > 9223372 THEN
            wide = TRUE
          END IF
        END IF
      END IF
    ELSE
      FAIL error(77050003, "not a decimal rate: " & text)
    END IF
    first = FALSE
  NEXT
  IF wholeDigits = 0 OR (point AND fractionDigits = 0) THEN
    FAIL error(77050003, "not a decimal rate: " & text)
  END IF
  IF negative OR NOT nonzero THEN
    FAIL error(77050002, "a rate must be above zero: " & text)
  END IF
  IF wide THEN
    FAIL error(77050010, "a rate must be at most 9223372.036854775807: " & text)
  END IF
  WHILE fractionDigits < 12
    fraction = fraction * 10
    fractionDigits = fractionDigits + 1
  END WHILE
  LET base AS Integer = whole * 1000000000000
  IF fraction > 9223372036854775807 - base THEN
    FAIL error(77050010, "a rate must be at most 9223372.036854775807: " & text)
  END IF
  RETURN Rate[scaled := base + fraction]
END FUNC"#;

/// The `String` overload of `money::toRate`; `func_to_rate` registers it.
pub(crate) fn implementation() -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "text",
            desc: "The rate as decimal text: digits, then optionally a `.` and up to 12 \
                   more digits, such as `\"0.842\"` or `\"1\"`.",
            aliases: &[],
            ty: ParameterType::String,
            default: DefaultValue::None,
        }],
        return_type: ParameterType::named(RATE_TYPE),
        errors: vec![
            "ErrInvalidFormat",
            "ErrInvalidArgument",
            "ErrOverflow",
            "ErrOutOfMemory",
        ],
        body: Body::mfb(BODY, "__money_toRateString"),
    }
}
