//! `money::toRate(Integer)` — the whole-number rate overload.
//!
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::RATE_TYPE;
use crate::codegen::registry::{Body, DefaultValue, Implementation, Parameter};
use crate::types::ParameterType;

/// The range checks come first, so the `*` below never overflows: `9223372` times
/// 10^12 is the largest whole rate that fits an `Integer`. The `*` is the checked
/// `Integer` multiply all the same.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __money_toRateInteger(value AS Integer) AS Rate
  IF value <= 0 THEN
    FAIL error(77050002, "a rate must be above zero: " & toString(value))
  END IF
  IF value > 9223372 THEN
    FAIL error(77050010, "a rate must be at most 9223372.036854775807: " & toString(value))
  END IF
  RETURN Rate[scaled := value * 1000000000000]
END FUNC"#;

/// The `Integer` overload of `money::toRate`; `func_to_rate` registers it.
pub(crate) fn implementation() -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "value",
            desc: "A whole-number rate, `1` through `9223372`.",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        }],
        return_type: ParameterType::named(RATE_TYPE),
        errors: vec!["ErrInvalidArgument", "ErrOverflow", "ErrOutOfMemory"],
        body: Body::mfb(BODY, "__money_toRateInteger"),
    }
}
