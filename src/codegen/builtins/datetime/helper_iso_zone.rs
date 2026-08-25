//! `__datetime_isoZone` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-77 D2: the `Z` token of the ISO pattern — "Z" for a zero UTC offset, else
' the signed offset label. Matches __datetime_formatToken's `Z`/runLen=1 arm so
' the hand-written __datetime_toIso stays byte-identical to the pattern formatter.
FUNC __datetime_isoZone(offset AS Integer) AS String
  IF offset = 0 THEN
    RETURN "Z"
  END IF
  RETURN __datetime_offsetLabel(offset)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_isoZone", BODY));
}
