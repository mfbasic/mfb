//! `__regex_scriptCanon` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-77 R2: canonicalize against the full generated Unicode script list
' (`__regex_scriptCanonName`, from third_party/unicode/Scripts-16.0.0.txt) instead
' of the 10 hand-listed scripts.
FUNC __regex_scriptCanon(low AS String) AS String
  RETURN __regex_scriptCanonName(low)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_scriptCanon", BODY));
}
