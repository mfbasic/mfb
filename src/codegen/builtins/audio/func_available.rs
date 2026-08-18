//! `audio::available` — frames a stream can move immediately (native OS-seam).
//!
//! Two overloads (`AudioInput` / `AudioOutput`) sharing one runtime symbol
//! (`audio.available`, branching on the handle kind at runtime).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID};

const INTRO: &str = r#"Frames an open stream can move immediately without blocking."#;
const DESC: &str = r#"`audio::available` returns how many whole frames a stream can move right now
without blocking, as an `Integer`. For an `AudioInput` it is the frames currently
readable; for an `AudioOutput` it is the frames writable before `audio::write` would
block. It is defined over both directions and never blocks. The result is never
negative — a device-reported negative count is clamped to `0`, as is a closed or
defaulted handle — so `available` is always safe to call. The untimed
`audio::poll(stream)` is exactly `audio::available(stream) > 0`."#;
const EX: &str = r#"Read exactly what is available, without blocking:

```
IMPORT audio

SUB main()
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  LET n = audio::available(mic)
  IF n > 0 THEN
    LET pcm = audio::read(mic, n)
  END IF
  audio::close(mic)
END SUB
```"#;

const STREAM_DESC: &str = "An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Borrowed, not consumed. A closed handle reports `0`.";

fn overload(stream_ty: &'static str) -> Implementation {
    Implementation {
        params: vec![param(
            "stream",
            STREAM_DESC,
            &[],
            ParameterType::Named(stream_ty),
        )],
        return_type: ParameterType::Integer,
        errors: vec!["ErrAudioUnavailable"],
        body: super::native_body(&[]),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "available",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("AudioInput or AudioOutput"),
        internal_only: false,
        implementations: vec![
            overload(AUDIO_INPUT_TYPE_ID),
            overload(AUDIO_OUTPUT_TYPE_ID),
        ],
    });
}
