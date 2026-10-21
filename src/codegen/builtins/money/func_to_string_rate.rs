//! `__money_rateToString` — the `toString(money::Rate)` override target.
//!
//! Registered via `add_helper` (private-only) and named by `add_override`, as
//! `toString(color::Color)` is; a program reaches it only by calling the general
//! `toString`. Body byte-significant (2-space indent → `.ncode` columns); do not
//! reformat.

use super::RATE_TYPE;
use crate::codegen::registry::{RegistryHelper, RegistryOverride, RegistryPackage};

/// The override's helper name, as `add_override` routes to it.
pub(crate) const RATE_TO_STRING: &str = "__money_rateToString";

/// Prints the value `scaled / 10^12` in decimal, with the fraction's trailing zeros
/// removed and no `.` when the fraction is zero. It does not validate: a record built
/// by hand prints what it holds, a negative one with a leading `-`.
///
/// Truncating `/` and `MOD` give a whole part and a fraction with the sign of
/// `scaled`; both are negated only after the split, because the negation of the
/// smallest `Integer` does not fit but the negation of either part does.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __money_rateToString(rate AS Rate) AS String
  MUT whole AS Integer = rate.scaled / 1000000000000
  MUT fraction AS Integer = rate.scaled MOD 1000000000000
  MUT sign AS String = ""
  IF rate.scaled < 0 THEN
    sign = "-"
    whole = 0 - whole
    fraction = 0 - fraction
  END IF
  IF fraction = 0 THEN
    RETURN sign & toString(whole)
  END IF
  MUT places AS Integer = 12
  WHILE fraction MOD 10 = 0
    fraction = fraction / 10
    places = places - 1
  END WHILE
  RETURN sign & toString(whole) & "." & strings::padLeft(toString(fraction), places, "0")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_override(RegistryOverride {
        builtin: "toString",
        arg_type: RATE_TYPE,
        helper: RATE_TO_STRING,
    });
    pkg.add_helper(RegistryHelper::always("money_rateToString", BODY));
}
