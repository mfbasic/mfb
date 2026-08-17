//! `__csv_needsQuote` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_needsQuote(field AS String, delimiter AS String, quote AS String) AS Boolean
  IF strings::contains(field, delimiter) THEN
    RETURN TRUE
  END IF
  IF strings::contains(field, quote) THEN
    RETURN TRUE
  END IF
  IF strings::contains(field, "\r") THEN
    RETURN TRUE
  END IF
  IF strings::contains(field, "\n") THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_needsQuote", BODY));
}
