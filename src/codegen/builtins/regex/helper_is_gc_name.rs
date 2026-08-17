//! `__regex_isGcName` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isGcName(name AS String) AS Boolean
  IF name = "Lu" OR name = "Ll" OR name = "Lt" OR name = "Lm" OR name = "Lo" THEN
    RETURN TRUE
  END IF
  IF name = "Mn" OR name = "Mc" OR name = "Me" THEN
    RETURN TRUE
  END IF
  IF name = "Nd" OR name = "Nl" OR name = "No" THEN
    RETURN TRUE
  END IF
  IF name = "Pc" OR name = "Pd" OR name = "Ps" OR name = "Pe" THEN
    RETURN TRUE
  END IF
  IF name = "Pi" OR name = "Pf" OR name = "Po" THEN
    RETURN TRUE
  END IF
  IF name = "Sm" OR name = "Sc" OR name = "Sk" OR name = "So" THEN
    RETURN TRUE
  END IF
  IF name = "Zs" OR name = "Zl" OR name = "Zp" THEN
    RETURN TRUE
  END IF
  IF name = "Cc" OR name = "Cf" OR name = "Cs" OR name = "Co" OR name = "Cn" THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isGcName", BODY));
}
