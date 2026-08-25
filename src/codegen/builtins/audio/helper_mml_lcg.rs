//! `__audio_mmlLcg` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Advance a 31-bit LCG (glibc constants) for deterministic noise.
FUNC __audio_mmlLcg(seed AS Integer) AS Integer
  RETURN bits::band(seed * 1103515245 + 12345, 2147483647)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlLcg", BODY));
}
