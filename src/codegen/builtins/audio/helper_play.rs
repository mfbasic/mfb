//! `__audio_play` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' audio::play(output, mml) — parse a single MML track and write it to the stream.
SUB __audio_play(RES out AS audio::AudioOutput, mml AS String)
  __audio_playSamples(out, __audio_mmlRenderSamples(mml))
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_play", BODY));
}
