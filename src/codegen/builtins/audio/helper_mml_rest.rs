//! `__audio_mmlRest` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Build a rest event of `frames` total frames.
FUNC __audio_mmlRest(frames AS Integer) AS __audio_MmlEvent
  RETURN __audio_MmlEvent[0.0, frames, 0, 0.0, 0, 0, 0]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlRest", BODY));
}
