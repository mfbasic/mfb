//! `audio::render` — synthesize one `AudioNote` to `s16le` PCM (source member).
//!
//! A pure-MFBASIC tone synthesizer (no device call): rewrites to the
//! `__audio_render` body in the injected source (`helper_render.rs`) through the
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

const INTRO: &str = r#"Synthesize one `AudioNote` to mono `s16le` PCM at 48 kHz."#;
const DESC: &str = r#"`audio::render` is a pure MFBASIC tone synthesizer, not a device call: it never
opens hardware and touches no audio stream. It turns one `AudioNote` into raw
single-channel `s16le` PCM at a fixed 48 kHz sample rate and returns it as a
`List OF Byte` — the same mono frame layout `audio::write` accepts, so the result can be handed straight to an open `AudioOutput`. The returned list is
`note.noteFrames * 2` bytes long (empty when `note.noteFrames <= 0`). Each frame is
a sine oscillator shaped by the note's `AudioEnvelope` (linear attack, decay to
`sustainLevel`, held sustain, linear release), scaled by `gainOverall`, converted to
an `Integer`, then clamped to the s16 range and encoded little-endian. `render` is
deterministic and platform-independent."#;
const EX: &str = r#"Render one second of A4 (440 Hz) and play it on the default output:

```
IMPORT audio

SUB main()
  LET env = AudioEnvelope[2400, 4800, 31200, 9600, 12000]
  LET note = AudioNote[440.0, 48000, env, 0.8]
  LET tone = audio::render(note)

  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::write(out, tone)
  audio::close(out)
END SUB
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
                "The note to synthesize: `frequencyHz`, `noteFrames`, an `AudioEnvelope`, and `gainOverall`. Construct it with `AudioNote[...]`.",
                &[],
                ParameterType::named(AUDIO_NOTE_TYPE),
            )],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrInvalidFormat", "ErrOverflow"],
            body: Body::mfb(BODY, "__audio_render"),
        }],
    });
}
