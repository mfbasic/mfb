//! `__astrings_flagFromMember` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_flagFromMember(m AS Integer) AS AttrTypeFlag
  IF m = 0 THEN
    RETURN AttrTypeFlag.Bold
  END IF
  IF m = 1 THEN
    RETURN AttrTypeFlag.Italic
  END IF
  IF m = 2 THEN
    RETURN AttrTypeFlag.Underline
  END IF
  IF m = 3 THEN
    RETURN AttrTypeFlag.Strike
  END IF
  RETURN AttrTypeFlag.Overline
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_flagFromMember", BODY));
}
