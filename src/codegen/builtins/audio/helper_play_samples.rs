//! `__audio_playSamples` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Encode the samples and write them to the open output stream. A no-op for empty
' PCM. Only a pointer to the stream is passed — the caller owns and closes it. The stream must be
' 48 kHz mono (what the sequencer renders).
SUB __audio_playSamples(RES out AS audio::AudioOutput, samples AS List OF Integer)
  IF len(samples) = 0 THEN
    EXIT SUB
  END IF
  audio::write(out, __audio_mmlEncode(samples))
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_playSamples", BODY));
}
