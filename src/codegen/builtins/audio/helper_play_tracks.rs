//! `__audio_playTracks` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' audio::play(output, tracks) — parse each track, mix them, and write to the
' stream. Each track is isolated: tempo/length/octave/volume/instrument do not
' carry between tracks.
SUB __audio_playTracks(RES out AS audio::AudioOutput, tracks AS List OF String)
  MUT rendered AS List OF List OF Integer = []
  FOR EACH tk IN tracks
    rendered = collections::append(rendered, __audio_mmlRenderSamples(tk))
  NEXT
  __audio_playSamples(out, __audio_mmlMix(rendered))
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_playTracks", BODY));
}
