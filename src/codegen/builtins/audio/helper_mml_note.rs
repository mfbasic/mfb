//! `__audio_mmlNote` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse a note token (`C`, `C+`, `D-`, `D16`, `C+8.`) into an event.
FUNC __audio_mmlNote(token AS String, octave AS Integer, length AS Integer, tempo AS Integer, volume AS Integer, wave AS Integer, staccato AS Boolean) AS __audio_MmlEvent
  LET semi AS Integer = __audio_mmlNoteSemitone(strings::mid(token, 0, 1))
  IF semi < 0 THEN
    FAIL error(77050002, "audio::play: unrecognized token '" & token & "'")
  END IF
  MUT idx AS Integer = 1
  MUT accidental AS Integer = 0
  IF idx < len(token) THEN
    LET c AS String = strings::mid(token, idx, 1)
    IF c = "+" THEN
      accidental = 1
      idx = idx + 1
    ELSEIF c = "-" THEN
      accidental = -1
      idx = idx + 1
    END IF
  END IF
  MUT digits AS String = ""
  WHILE idx < len(token) AND __audio_mmlIsDigit(strings::mid(token, idx, 1))
    digits = digits & strings::mid(token, idx, 1)
    idx = idx + 1
  END WHILE
  MUT noteLen AS Integer = length
  IF len(digits) > 0 THEN
    noteLen = __audio_mmlParseUint(digits)
    IF noteLen < 1 OR noteLen > 64 THEN
      FAIL error(77050002, "audio::play: note length out of range in '" & token & "'")
    END IF
  END IF
  LET dots AS Integer = __audio_mmlTrailingDots(token, idx)
  IF dots < 0 THEN
    FAIL error(77050002, "audio::play: unrecognized token '" & token & "'")
  END IF
  LET midi AS Integer = 12 * (octave + 1) + semi + accidental
  LET freq AS Float = 440.0 * math::pow(2.0, toFloat(midi - 69) / 12.0)
  LET total AS Integer = __audio_mmlFrames(tempo, noteLen, dots)
  MUT sound AS Integer = total
  IF staccato THEN
    sound = total / 2
    IF sound < 1 THEN
      sound = 1
    END IF
  END IF
  RETURN __audio_MmlEvent[freq, total, sound, toFloat(volume) / 10.0, wave, 48, 48]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlNote", BODY));
}
