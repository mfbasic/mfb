//! `__regex_propMatchItem` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_propMatchItem(name AS String, neg AS Boolean, cp AS Integer) AS Boolean
  LET hit AS Boolean = __regex_propTest(name, cp)
  IF neg THEN
    RETURN NOT hit
  END IF
  RETURN hit
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_propMatchItem", BODY));
}
