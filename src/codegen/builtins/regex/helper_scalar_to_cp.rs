//! `__regex_scalarToCp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Code point of a single scalar string via the shared UTF-32 encoder.
FUNC __regex_scalarToCp(ch AS String) AS Integer
  IF ch = "" THEN
    RETURN 0
  END IF
  LET cps AS List OF Integer = encoding::utf32Encode(ch)
  RETURN collections::get(cps, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_scalarToCp", BODY));
}
