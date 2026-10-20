//! `audio::render` — synthesize one `AudioNote` to `s16le` PCM (source member).
//!
//! A pure-MFBASIC tone synthesizer (no device call): rewrites to the
//! `__audio_render` body in the injected source (`BODY` below) through the
//! generic `registry::rewrite_target`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_NOTE_TYPE};

#[rustfmt::skip]
const BODY: &str =
r#"' audio::render — render a note to mono s16le PCM at 48 kHz (the frame layout the
' playback surface consumes: one frame is 2 bytes). A sine oscillator shaped by
' the note's attack/decay/sustain/release envelope; the release is the final
' `releaseFrames` and the middle is held at `sustainLevel`.
'
' The little-endian two-byte emit is INLINE for the same reason as
' `__audio_mmlEncode`: passing the growing `pcm` to a helper copies the whole
' accumulator per sample and makes the render O(n^2).
FUNC __audio_render(note AS AudioNote) AS List OF Byte
  MUT pcm AS List OF Byte = []
  LET n AS Integer = note.noteFrames
  ' A non-positive count is the documented empty render. Returning before the loop
  ' also keeps `n - 1` from overflowing at the most negative Integer.
  IF n <= 0 THEN
    RETURN pcm
  END IF
  ' Bounded before a frame is rendered (bug-810): `audio::play`'s ten-minute limit,
  ' 28.8 million frames at 48 kHz (a 57.6 MB result). Unbounded, a huge `noteFrames`
  ' appended two bytes a frame until memory ran out.
  IF n > 28800000 THEN
    FAIL error(77050002, "audio::render: note is longer than the 10 minute limit")
  END IF
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
    MUT v AS Integer = sample
    IF v < 0 THEN
      v = v + 65536
    END IF
    pcm = collections::append(pcm, toByte(bits::band(v, 255)))
    pcm = collections::append(pcm, toByte(bits::band(bits::sr(v, 8), 255)))
  NEXT
  RETURN pcm
END FUNC"#;

const INTRO: &str = r#"Synthesize one `audio::AudioNote` to mono `s16le` PCM at 48 kHz."#;
const DESC: &str = r#"`audio::render` is a pure MFBASIC tone synthesizer, not a device call: it never
opens hardware and touches no audio stream. It turns one `audio::AudioNote` into raw
single-channel `s16le` PCM at a fixed 48 kHz sample rate and returns it as a
`List OF Byte` — the same mono frame layout `audio::write` accepts, so the result can be handed straight to an open `AudioOutput`. The returned list is
`note.noteFrames * 2` bytes long (empty when `note.noteFrames <= 0`). A note longer
than ten minutes — `note.noteFrames` above 28,800,000 — is refused with
`ErrInvalidArgument` before anything is rendered, the same limit `audio::play`
applies to a track. Each frame is
a sine oscillator shaped by the note's `audio::AudioEnvelope` (linear attack, decay to
`sustainLevel`, held sustain, linear release), scaled by `gainOverall`, converted to
an `Integer`, then clamped to the s16 range and encoded little-endian. `render` is
deterministic and platform-independent. A `note.frequencyHz` of `NaN` reaches the
sine calculation as `NaN` and raises `ErrFloatNaN`. `ErrOutOfMemory` means the
program ran out of memory."#;
const EX: &str = r#"Render one second of A4 (440 Hz) and play it on the default output:

```
IMPORT audio

FUNC main() AS Nothing
  LET env = audio::AudioEnvelope[2400, 4800, 31200, 9600, 12000]
  LET note = audio::AudioNote[440.0, 48000, env, 0.8]
  LET tone = audio::render(note)

  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::write(out, tone)
  audio::close(out)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "render",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![param(
                "note",
                "The note to synthesize: `frequencyHz`, `noteFrames`, an `audio::AudioEnvelope`, and `gainOverall`. Construct it with `audio::AudioNote[...]`.",
                &[],
                ParameterType::named(AUDIO_NOTE_TYPE),
            )],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrInvalidArgument", "ErrInvalidFormat", "ErrOverflow", "ErrFloatNaN", "ErrOutOfMemory"],
            body: Body::mfb(BODY, "__audio_render"),
        }],
    });
}
