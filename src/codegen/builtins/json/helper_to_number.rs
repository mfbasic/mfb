//! `__json_toNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-A: the TRAP used to discard `toFloat`'s diagnosis and re-fail with the
' generic 77050003, so a document whose number is simply outside Float's range
' ("1e400" -> 77050010 ErrOverflow) was indistinguishable from a document with a
' syntax error. The grammar was already checked upstream by `__json_validNumber`,
' so anything reaching this TRAP is a RANGE or precision verdict from `toFloat`
' and that verdict is what the caller needs. Re-raise it with json's context
' prepended, keeping `err.code` intact.
FUNC __json_toNumber(value AS String) AS Float
  RETURN toFloat(value)
  TRAP(err)
    FAIL error(err.code, "JSON number " & value & " is not representable: " & err.message)
  END TRAP
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_toNumber", BODY));
}
