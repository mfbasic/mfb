//! `__json_roundTrips` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-G: does this candidate rendering read back as the value?
'
' The search tries short renderings first, and a short one can name a number
' too large for a Float even though the value it is approximating is finite:
' the first candidate for 1.7976931348623157e308 is `2e+308`, which overflows.
' `toFloat` raises on overflow rather than saturating, so the check has to
' absorb that -- a candidate that does not denote a Float plainly does not
' round-trip, and the search should move on to a longer one rather than fail
' the whole serialization.
'
' Underflow needs no special handling: a too-small candidate converts to zero,
' compares unequal, and is rejected the ordinary way.
FUNC __json_roundTrips(text AS String, value AS Float) AS Boolean
  RETURN toFloat(text) = value
TRAP(e)
  RETURN FALSE
END TRAP
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_roundTrips", BODY));
}
