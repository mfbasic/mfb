//! `__csv_firstCode` — shared private helper for the `csv` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' First Unicode scalar code of a (single-character) dialect string. An empty
' string would make parsing ambiguous, so it is rejected.
FUNC __csv_firstCode(s AS String) AS Integer
  LET codes AS List OF Integer = encoding::utf32Encode(s)
  IF len(codes) = 0 THEN
    FAIL error(77050003, "csv: dialect delimiter/quote must be a non-empty character")
  END IF
  RETURN collections::get(codes, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_firstCode", BODY));
}
