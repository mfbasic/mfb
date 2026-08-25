//! `__audio_mmlRenderSamples` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Render one MML track to signed s16 samples.
FUNC __audio_mmlRenderSamples(mml AS String) AS List OF Integer
  RETURN __audio_mmlSynth(__audio_mmlParse(mml))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlRenderSamples", BODY));
}
