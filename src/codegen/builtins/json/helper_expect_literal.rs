//! `__json_expectLiteral` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_expectLiteral(chars AS List OF String, index AS Integer, literal AS String) AS Integer
  LET target AS List OF String = strings::graphemes(literal)
  RETURN __json_expectLiteralAt(chars, index, target, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_expectLiteral", BODY));
}
