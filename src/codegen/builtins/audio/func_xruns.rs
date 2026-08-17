//! `audio::xruns` — cumulative overrun/underrun count for a stream (native OS-seam).
//!
//! Two overloads (`AudioInput` / `AudioOutput`) sharing one runtime symbol
//! (`audio.xruns`, branching on the handle kind at runtime). Docs migrated from
//! `src/docs/man/builtins/audio/xruns.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID};

const INTRO: &str =
    r#"Cumulative count of overrun/underrun events on a stream since it was opened."#;
const DESC: &str = r#"`audio::xruns` returns, as an `Integer`, the number of xrun events recorded on an
open stream since it was opened: capture overruns for an `AudioInput`, playback
underruns for an `AudioOutput`. The value is a monotonic counter, incremented by
exactly one per xrun event; it counts events, not lost frames. A stream that has
never dropped audio reports `0`. `xruns` cannot fail: it takes no library call (so
it never raises `ErrAudioUnavailable` even on a Linux host without ALSA), and a
closed or defaulted handle reports `0`. Both directions share one internal body; the
direction is read from the handle at runtime."#;
const EX: &str = r#"Check for lost audio after a playback loop:

```
IMPORT audio
IMPORT io

SUB main()
  RES out AS audio::AudioOutput = audio::openOutput(48000, 2, 512)
  LET pcm AS List OF Byte = [0, 0, 0, 0]
  audio::write(out, pcm)
  io::print("underruns: " & toString(audio::xruns(out)))
  audio::close(out)
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
        errors: vec![],
        body: super::native_body(&[]),
    }
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "xruns",
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
