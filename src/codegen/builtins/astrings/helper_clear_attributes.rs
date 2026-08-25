//! `__astrings_clearAttributes` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_clearAttributes(a AS AttributedString) AS AttributedString
  MUT empty AS List OF AttrSpan = []
  RETURN astrings::writeSpans(a, empty)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_clearAttributes", BODY));
}
