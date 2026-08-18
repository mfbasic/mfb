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

const INTRO: &str = r#"Parse MML music text and play it on an open output stream."#;
const DESC: &str = r#"`audio::play` is a small MML (Music Macro Language) sequencer. It parses one or
more tracks of MML text, synthesizes each track to mono `s16le` PCM at a fixed 48 kHz
sample rate, mixes the tracks by summing (with clamping), and writes the result to
`output` via `audio::write`. Because the sequencer renders at 48 kHz mono, `output`
must be an `AudioOutput` opened with `sampleRate = 48000` and `channels = 1`. `play`
parses and synthesizes the entire program before writing, so malformed MML raises an
error and nothing is written. The `output` stream is borrowed — not consumed, so the
caller keeps ownership and must close it. A track is a string of space-separated
tokens (notes `A`..`G` with accidentals/length/dots, `R`/`P` rests, `O`/`<`/`>`
octave, `L` length, `T` tempo, `V` volume, `I <name>` instrument, `( )` legato,
`[ ]` staccato, `{ }<count>` repeat). `play` is deterministic."#;
const EX: &str = r#"Play a bass line and a lead together on the same stream:

```
IMPORT audio

SUB main()
  LET bass = "T100 O2 L4 I triangle { C G }4"
  LET lead = "T100 O4 L8 I sine C E G < C > [ C E G ] { C. D16 }2"

  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::play(out, [bass, lead])
  audio::close(out)
END SUB
```"#;

fn output_param() -> Parameter {
    param(
        "output",
        "An open playback stream opened at 48 kHz mono (`audio::openOutput(48000, 1, ...)`). Borrowed — `play` writes to it and leaves it open.",
        &[],
        ParameterType::Named(AUDIO_OUTPUT_TYPE_ID),
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
                body: Body::Rewrite("__audio_playTracks"),
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
                body: Body::Rewrite("__audio_play"),
            },
        ],
    });
}
