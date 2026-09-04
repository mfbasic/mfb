//! `audio::play` — parse MML music text and play it (source member).
//!
//! Two overloads selected on the second argument's type: a single `String` track
//! rewrites to `__audio_play`, a `List OF String` of tracks to `__audio_playTracks`
//! (both source-companion bodies, reached through the generic
//! `registry::rewrite_target`).

use crate::codegen::registry::{
    Body, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use super::{param, AUDIO_OUTPUT_TYPE_ID};

#[rustfmt::skip]
const BODY_TRACK: &str =
r#"' audio::play(output, mml) — parse a single MML track and write it to the stream.
SUB __audio_play(RES out AS audio::AudioOutput, mml AS String)
  __audio_playSamples(out, __audio_mmlRenderSamples(mml))
END SUB"#;

#[rustfmt::skip]
const BODY_TRACKS: &str =
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

const INTRO: &str = r#"Parse MML music text and play it on an open output stream."#;
const DESC: &str = r#"`audio::play` is a small MML (Music Macro Language) sequencer. It parses one or
more tracks of MML text, synthesizes each track to mono `s16le` PCM at a fixed 48 kHz
sample rate, mixes the tracks by summing (with clamping), and writes the result to
`output` via `audio::write`. Because the sequencer renders at 48 kHz mono, `output`
must be an `AudioOutput` opened with `sampleRate = 48000` and `channels = 1`. `play`
parses and synthesizes the entire program before writing, so malformed MML raises an
error and nothing is written. The `output` stream stays open — you still
close it. A track is a string of space-separated
tokens (notes `A`..`G` with accidentals/length/dots, `R`/`P` rests, `O`/`<`/`>`
octave, `L` length, `T` tempo, `V` volume, `I <name>` instrument, `( )` legato,
`[ ]` staccato, `{ }<count>` repeat). A track is refused with `ErrInvalidArgument`
when its repeats expand past 65,536 tokens or it would play for longer than ten
minutes. `play` is deterministic."#;
const EX: &str = r#"Play a bass line and a lead together on the same stream:

```
IMPORT audio

SUB main()
  LET bass = "T180 O2 L8 I triangle C G"
  LET lead = "T180 O4 L16 I sine C E G [ C E G ]"

  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::play(out, [bass, lead])
  audio::close(out)
END SUB
```"#;

fn output_param() -> Parameter {
    param(
        "output",
        "An open playback stream opened at 48 kHz mono (`audio::openOutput(48000, 1, ...)`). The handle stays open — `play` writes to it and leaves it open.",
        &[],
        ParameterType::named(AUDIO_OUTPUT_TYPE_ID),
    )
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let errors = || {
        vec![
            "ErrInvalidArgument",
            "ErrAudioDevice",
            "ErrAudioUnavailable",
        ]
    };
    pkg.add_function(RegistryFunction {
        name: "play",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("AudioOutput, String or AudioOutput, List OF String"),
        // The `List OF String` (multi-track) overload MUST be listed first: the
        // registry's lenient dispatch is coarse on a scalar `String` PARAMETER (it
        // accepts any argument, including a `List`), but a CONTAINER pattern
        // (`ListOf`) is matched structurally and correctly rejects a `String`
        // argument. So the specific list overload is tried first and the scalar
        // `String` overload is the catch-all — reproducing the pre-migration
        // `source_implementation_name` selection (`play(out, [tracks])` →
        // `__audio_playTracks`, `play(out, "mml")` → `__audio_play`).
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    output_param(),
                    param(
                        "tracks",
                        "Several MML tracks played together (multi-track overload).",
                        &["mml"],
                        ParameterType::list_of(ParameterType::String),
                    ),
                ],
                return_type: ParameterType::Nothing,
                errors: errors(),
                body: Body::mfb(BODY_TRACKS, "__audio_playTracks"),
            },
            Implementation {
                params: vec![
                    output_param(),
                    param(
                        "mml",
                        "A single MML track (single-track overload).",
                        &["tracks"],
                        ParameterType::String,
                    ),
                ],
                return_type: ParameterType::Nothing,
                errors: errors(),
                body: Body::mfb(BODY_TRACK, "__audio_play"),
            },
        ],
    });
}
