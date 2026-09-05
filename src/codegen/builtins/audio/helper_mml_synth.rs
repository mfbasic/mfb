//! `__audio_mmlSynth` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Synthesize events into a list of signed s16 samples.
FUNC __audio_mmlSynth(events AS List OF __audio_MmlEvent) AS List OF Integer
  ' Bounded before a frame is rendered (bug-509, DEC-55). Ten minutes at 48 kHz is
  ' 28.8 million samples, held twice over (synthesised, then encoded), and `play`
  ' blocks for the tune's real-time length regardless. The token cap alone does not
  ' bound this: 5,000 whole notes at T32 are ten hours of audio.
  MUT totalFrames AS Integer = 0
  FOR EACH ev IN events
    totalFrames = totalFrames + ev.totalFrames
  NEXT
  IF totalFrames > 28800000 THEN
    FAIL error(77050002, "audio::play: track is longer than the 10 minute limit")
  END IF
  MUT out AS List OF Integer = []
  FOR EACH ev IN events
    IF ev.freq <= 0.0 THEN
      FOR i = 0 TO ev.totalFrames - 1
        out = collections::append(out, 0)
      NEXT
    ELSE
      LET sound AS Integer = ev.soundFrames
      LET fadeIn AS Integer = __audio_mmlClampFade(ev.fadeIn, sound)
      LET fadeOut AS Integer = __audio_mmlClampFade(ev.fadeOut, sound)
      MUT noiseSeed AS Integer = 305419896
      FOR i = 0 TO ev.totalFrames - 1
        MUT si AS Integer = 0
        IF i < sound THEN
          LET t AS Float = toFloat(i) / 48000.0
          LET phase AS Float = ev.freq * t
          LET frac AS Float = phase - toFloat(math::floor(phase))
          MUT w AS Float = 0.0
          IF ev.wave = 0 THEN
            w = math::sin(2.0 * math::pi * phase)
          ELSEIF ev.wave = 1 THEN
            IF frac < 0.5 THEN
              w = 1.0
            ELSE
              w = -1.0
            END IF
          ELSEIF ev.wave = 2 THEN
            w = 4.0 * math::abs(frac - 0.5) - 1.0
          ELSEIF ev.wave = 3 THEN
            w = 2.0 * frac - 1.0
          ELSE
            noiseSeed = __audio_mmlLcg(noiseSeed)
            w = toFloat(noiseSeed MOD 65536) / 32768.0 - 1.0
          END IF
          MUT amp AS Float = 1.0
          IF i < fadeIn THEN
            amp = toFloat(i) / toFloat(fadeIn)
          ELSEIF i >= sound - fadeOut THEN
            amp = toFloat(sound - i) / toFloat(fadeOut)
          END IF
          si = __audio_clampS16(toInt(w * amp * ev.gain * 32767.0))
        END IF
        out = collections::append(out, si)
      NEXT
    END IF
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlSynth", BODY));
}
