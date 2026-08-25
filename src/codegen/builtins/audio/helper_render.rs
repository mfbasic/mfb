//! `__audio_render` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' audio::render — render a note to mono s16le PCM at 48 kHz (the frame layout the
' playback surface consumes: one frame is 2 bytes). A sine oscillator shaped by
' the note's attack/decay/sustain/release envelope; the release is the final
' `releaseFrames` and the middle is held at `sustainLevel`.
FUNC __audio_render(note AS AudioNote) AS List OF Byte
  MUT pcm AS List OF Byte = []
  LET n AS Integer = note.noteFrames
  LET a AS Integer = note.envelope.attackFrames
  LET d AS Integer = note.envelope.decayFrames
  LET r AS Integer = note.envelope.releaseFrames
  LET peak AS Float = 32767.0
  LET sustain AS Float = toFloat(note.envelope.sustainLevel)
  LET releaseStart AS Integer = n - r
  FOR i = 0 TO n - 1
    LET t AS Float = toFloat(i) / 48000.0
    LET wave AS Float = math::sin(2.0 * math::pi * note.frequencyHz * t)
    MUT env AS Float = sustain
    IF i < a THEN
      env = peak * (toFloat(i) / toFloat(a))
    ELSEIF i < a + d THEN
      env = peak + (sustain - peak) * (toFloat(i - a) / toFloat(d))
    ELSEIF i >= releaseStart THEN
      env = sustain * (toFloat(n - i) / toFloat(r))
    END IF
    LET sample AS Integer = __audio_clampS16(toInt(wave * env * note.gainOverall))
    pcm = __audio_appendS16LE(pcm, sample)
  NEXT
  RETURN pcm
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_render", BODY));
}
