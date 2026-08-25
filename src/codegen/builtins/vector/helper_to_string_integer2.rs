//! `__vector_toString_integer2` — shared private helper for the `vector` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __vector_toString_integer2(v AS Integer2) AS String
  RETURN "(" & toString(v.x) & ", " & toString(v.y) & ")"
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("vector_toString_integer2", BODY));
}
