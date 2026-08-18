//! `__regex_lookupName` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_lookupName(name AS String, r AS __regex_Result, value AS String, prog AS __regex_Program) AS String
  IF collections::hasKey(prog.names, name) THEN
    RETURN __regex_lookupNum(collections::get(prog.names, name), r, value, prog)
  END IF
  RETURN ""
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_lookupName", BODY));
}
