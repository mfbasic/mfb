//! `__datetime_isLetter` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_isLetter(ch AS String) AS Boolean
  RETURN (ch >= "A" AND ch <= "Z") OR (ch >= "a" AND ch <= "z")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_isLetter", BODY));
}
