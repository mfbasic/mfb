//! `__net_slice` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Half-open grapheme slice `s[start, stop)`.
FUNC __net_slice(s AS String, start AS Integer, stop AS Integer) AS String
  IF stop <= start THEN
    RETURN ""
  END IF
  RETURN strings::mid(s, start, stop - start)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_slice", BODY));
}
