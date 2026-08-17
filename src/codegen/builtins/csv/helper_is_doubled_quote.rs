//! `__csv_isDoubledQuote` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_isDoubledQuote(chars AS List OF Integer, count AS Integer, index AS Integer, quoteCode AS Integer) AS Boolean
  IF index + 1 >= count THEN
    RETURN FALSE
  END IF
  IF collections::get(chars, index + 1) = quoteCode THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_isDoubledQuote", BODY));
}
