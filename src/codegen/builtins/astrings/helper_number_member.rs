//! `__astrings_numberMember` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Number attributes share `class=2`, so — like flags — they need a member
REM ordinal to stay distinguishable in storage. `FontSize` keeps member 0 (its
REM historical encoding, so pre-existing stored spans still decode as FontSize).
FUNC __astrings_numberMember(t AS AttrTypeNumber) AS Integer
  MATCH t
    CASE AttrTypeNumber.FontSize
      RETURN 0
    CASE AttrTypeNumber.Foreground
      RETURN 1
    CASE AttrTypeNumber.Background
      RETURN 2
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_numberMember", BODY));
}
