//! `__regex_lookupNum` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_lookupNum(num AS Integer, r AS __regex_Result, value AS String, prog AS __regex_Program) AS String
  IF num < 0 OR num > prog.groups THEN
    RETURN ""
  END IF
  LET s AS Integer = collections::get(r.caps, 2 * num)
  LET e AS Integer = collections::get(r.caps, 2 * num + 1)
  IF s < 0 OR e < 0 THEN
    RETURN ""
  END IF
  RETURN strings::mid(value, s, e - s)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_lookupNum", BODY));
}
