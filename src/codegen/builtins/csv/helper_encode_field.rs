//! `__csv_encodeField` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_encodeField(field AS String, delimiter AS String, quote AS String) AS String
  IF __csv_needsQuote(field, delimiter, quote) THEN
    RETURN __csv_quoteField(field, quote)
  END IF
  RETURN field
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_encodeField", BODY));
}
