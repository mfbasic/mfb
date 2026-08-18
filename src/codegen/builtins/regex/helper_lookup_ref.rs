//! `__regex_lookupRef` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_lookupRef(ref AS String, r AS __regex_Result, value AS String, prog AS __regex_Program) AS String
  IF __regex_allDigits(ref) THEN
    RETURN __regex_lookupNum(__regex_parseIntClamp(ref), r, value, prog)
  END IF
  RETURN __regex_lookupName(ref, r, value, prog)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_lookupRef", BODY));
}
