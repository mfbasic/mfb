//! `__audio_mmlParse` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse one MML track into a list of events. Raises on invalid MML.
FUNC __audio_mmlParse(mml AS String) AS List OF __audio_MmlEvent
  LET flat AS List OF String = __audio_mmlTokens(mml)
  MUT events AS List OF __audio_MmlEvent = []
  MUT tempo AS Integer = 120
  MUT octave AS Integer = 4
  MUT length AS Integer = 4
  MUT volume AS Integer = 10
  MUT wave AS Integer = 0
  MUT expectName AS Boolean = FALSE
  MUT inLegato AS Boolean = FALSE
  MUT inStaccato AS Boolean = FALSE
  MUT legatoStart AS Integer = 0
  FOR EACH tk IN flat
    IF expectName THEN
      LET wc AS Integer = __audio_mmlWaveCode(tk)
      IF wc < 0 THEN
        FAIL error(77050002, "audio::play: unknown instrument '" & tk & "'")
      END IF
      wave = wc
      expectName = FALSE
    ELSEIF tk = "I" THEN
      expectName = TRUE
    ELSEIF tk = "(" THEN
      IF inLegato OR inStaccato THEN
        FAIL error(77050002, "audio::play: legato may not nest inside legato or staccato")
      END IF
      inLegato = TRUE
      legatoStart = len(events)
    ELSEIF tk = ")" THEN
      IF NOT inLegato THEN
        FAIL error(77050002, "audio::play: ')' without matching '('")
      END IF
      inLegato = FALSE
      events = __audio_mmlApplyLegato(events, legatoStart)
    ELSEIF tk = "[" THEN
      IF inLegato OR inStaccato THEN
        FAIL error(77050002, "audio::play: staccato may not nest inside legato or staccato")
      END IF
      inStaccato = TRUE
    ELSEIF tk = "]" THEN
      IF NOT inStaccato THEN
        FAIL error(77050002, "audio::play: ']' without matching '['")
      END IF
      inStaccato = FALSE
    ELSEIF tk = "<" THEN
      octave = octave - 1
      IF octave < 0 THEN
        octave = 0
      END IF
    ELSEIF tk = ">" THEN
      octave = octave + 1
      IF octave > 6 THEN
        octave = 6
      END IF
    ELSEIF strings::mid(tk, 0, 1) = "T" THEN
      tempo = __audio_mmlReqInt(tk, 32, 255, "tempo")
    ELSEIF strings::mid(tk, 0, 1) = "O" THEN
      octave = __audio_mmlReqInt(tk, 0, 6, "octave")
    ELSEIF strings::mid(tk, 0, 1) = "L" THEN
      length = __audio_mmlReqInt(tk, 1, 64, "length")
    ELSEIF strings::mid(tk, 0, 1) = "V" THEN
      volume = __audio_mmlReqInt(tk, 0, 10, "volume")
    ELSEIF strings::mid(tk, 0, 1) = "P" THEN
      LET pauseLen AS Integer = __audio_mmlReqInt(tk, 1, 64, "pause")
      events = collections::append(events, __audio_mmlRest(__audio_mmlFrames(tempo, pauseLen, 0)))
    ELSEIF strings::mid(tk, 0, 1) = "R" THEN
      LET dots AS Integer = __audio_mmlTrailingDots(tk, 1)
      IF dots < 0 THEN
        FAIL error(77050002, "audio::play: unrecognized token '" & tk & "'")
      END IF
      events = collections::append(events, __audio_mmlRest(__audio_mmlFrames(tempo, length, dots)))
    ELSE
      events = collections::append(events, __audio_mmlNote(tk, octave, length, tempo, volume, wave, inStaccato))
    END IF
  NEXT
  IF expectName THEN
    FAIL error(77050002, "audio::play: 'I' with no instrument name")
  END IF
  IF inLegato THEN
    FAIL error(77050002, "audio::play: unclosed legato '('")
  END IF
  IF inStaccato THEN
    FAIL error(77050002, "audio::play: unclosed staccato '['")
  END IF
  RETURN events
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlParse", BODY));
}
