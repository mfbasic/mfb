//! `__regex_scriptTest` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-77 R2: a scalar is in script `name` iff its generated Script property
' (`regex::scriptOf`) equals `name`.
FUNC __regex_scriptTest(name AS String, cp AS Integer) AS Boolean
  RETURN regex::scriptOf(cp) = name
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_scriptTest", BODY));
}
