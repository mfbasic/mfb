//! `__json_requireFiniteNumberText` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-A: was `__json_isInvalidNumberText`, a Boolean predicate whose two
' callers both answered a TRUE with the generic 77050003. JSON has no syntax for
' a non-finite number, so `stringify` must refuse one -- but "this is not JSON"
' and "this number is a NaN" are different diagnoses, and only the second tells
' the caller to look at their arithmetic. The formatter renders a non-finite
' Float as one of these four texts, so the text is the detector.
SUB __json_requireFiniteNumberText(value AS String)
  IF value = "nan" THEN
    FAIL error(77050013, "cannot serialize a NaN as JSON: JSON has no non-finite numbers")
  END IF
  IF value = "-nan" THEN
    FAIL error(77050013, "cannot serialize a NaN as JSON: JSON has no non-finite numbers")
  END IF
  IF value = "inf" THEN
    FAIL error(77050014, "cannot serialize an infinity as JSON: JSON has no non-finite numbers")
  END IF
  IF value = "-inf" THEN
    FAIL error(77050014, "cannot serialize an infinity as JSON: JSON has no non-finite numbers")
  END IF
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_requireFiniteNumberText", BODY));
}
