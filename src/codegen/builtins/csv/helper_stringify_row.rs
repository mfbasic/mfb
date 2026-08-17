//! `__csv_stringifyRow` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_stringifyRow(row AS List OF String, delimiter AS String, quote AS String) AS String
  MUT out AS String = ""
  MUT firstField AS Boolean = TRUE
  FOR EACH field IN row
    IF firstField THEN
      firstField = FALSE
    ELSE
      out = out & delimiter
    END IF
    out = out & __csv_encodeField(field, delimiter, quote)
  NEXT
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_stringifyRow", BODY));
}
