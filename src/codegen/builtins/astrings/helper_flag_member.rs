//! `__astrings_flagMember` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_flagMember(t AS AttrTypeFlag) AS Integer
  MATCH t
    CASE AttrTypeFlag.Bold
      RETURN 0
    CASE AttrTypeFlag.Italic
      RETURN 1
    CASE AttrTypeFlag.Underline
      RETURN 2
    CASE AttrTypeFlag.Strike
      RETURN 3
    CASE AttrTypeFlag.Overline
      RETURN 4
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_flagMember", BODY));
}
