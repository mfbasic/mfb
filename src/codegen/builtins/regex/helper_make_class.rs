//! `__regex_makeClass` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_makeClass(neg AS Boolean, fold AS Boolean, items AS List OF __regex_ClassItem) AS __regex_Class
  RETURN __regex_Class[neg, fold, items, __regex_asciiClassBitset(items, fold)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_makeClass", BODY));
}
