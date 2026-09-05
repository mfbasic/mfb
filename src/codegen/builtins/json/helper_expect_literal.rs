//! `__json_expectLiteral` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_expectLiteral(bytes AS List OF Byte, index AS Integer, literal AS String) AS Integer
  LET target AS List OF Byte = strings::toBytes(literal)
  RETURN __json_expectLiteralAt(bytes, index, target, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_expectLiteral", BODY));
}
