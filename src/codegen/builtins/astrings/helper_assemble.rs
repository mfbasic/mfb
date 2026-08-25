//! `__astrings_assemble` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Assemble a new AttributedString from transformed text and remapped spans.
FUNC __astrings_assemble(newText AS String, spans AS List OF AttrSpan) AS AttributedString
  RETURN astrings::writeSpans(astrings::fromString(newText), spans)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_assemble", BODY));
}
