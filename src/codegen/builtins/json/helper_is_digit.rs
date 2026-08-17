//! `__json_isDigit` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 E2: the ASCII digit test, spelled as the range compare used by regex and
' datetime (was a strings::contains substring search). ch is always a single
' scanned character here. Distinct from strings::isDigit, which is Unicode-Nd.
FUNC __json_isDigit(ch AS String) AS Boolean
  RETURN ch >= "0" AND ch <= "9"
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_isDigit", BODY));
}
