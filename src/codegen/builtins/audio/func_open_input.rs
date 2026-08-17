//! `audio::openInput` — open a capture stream (native OS-seam).
//!
//! Two overloads: the default-device form (`sampleRate, channels, bufferFrames`)
//! and the named-device form (a leading `AudioDevice`), which declares the
//! code-form alias `openInputDevice` (`builder_values` rewrites the device-first
//! NIR call to it). Docs migrated from `src/docs/man/builtins/audio/openInput.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_DEVICE_TYPE, AUDIO_INPUT_TYPE_ID};

const INTRO: &str = r#"Open a capture stream and return an `AudioInput` handle."#;
const DESC: &str = r#"`audio::openInput` opens a PCM capture stream and returns an `AudioInput`. The
three-argument form opens the system default input device; the four-argument form
opens the specific device named by an `AudioDevice` from `audio::devices()`. The
stream delivers raw interleaved `s16le` PCM (one frame is `channels * 2` bytes).
`sampleRate` must be in `8000..=192000`, `channels` must be `1` or `2`, and
`bufferFrames` must be in `64..=8192`; any value outside these bounds raises
`ErrInvalidArgument`. The returned `AudioInput` is a move-only, non-sendable
resource closed by lexical drop or `audio::close`; read from it with `audio::read`,
which is defined only over `AudioInput`."#;
const EX: &str = r#"Capture 100 ms of mono audio at 48 kHz from the default input:

```
IMPORT audio

SUB main()
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  LET pcm = audio::read(mic, 4800)
  audio::close(mic)
END SUB
```"#;

const SAMPLE_RATE: &str = "Capture rate in Hz. Must be in `8000..=192000`.";
const CHANNELS: &str = "Channel count: `1` (mono) or `2` (stereo).";
const BUFFER_FRAMES: &str = "Frames per OS buffer. Must be in `64..=8192`.";
const DEVICE: &str = "The device to open, from `audio::devices()` (four-argument form only). A device whose `id` no longer exists raises `ErrAudioDevice`.";

pub(super) fn register(pkg: &mut RegistryPackage) {
    let errors = || {
        vec![
            "ErrInvalidArgument",
            "ErrAudioUnavailable",
            "ErrAudioDevice",
            "ErrOutOfMemory",
        ]
    };
    pkg.add_function(RegistryFunction {
        name: "openInput",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some(
            "Integer, Integer, Integer or AudioDevice, Integer, Integer, Integer",
        ),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    param("sampleRate", SAMPLE_RATE, &[], ParameterType::Integer),
                    param("channels", CHANNELS, &[], ParameterType::Integer),
                    param("bufferFrames", BUFFER_FRAMES, &[], ParameterType::Integer),
                ],
                return_type: ParameterType::Named(AUDIO_INPUT_TYPE_ID),
                errors: errors(),
                body: super::native_body(&[]),
            },
            Implementation {
                params: vec![
                    param(
                        "device",
                        DEVICE,
                        &[],
                        ParameterType::Named(AUDIO_DEVICE_TYPE),
                    ),
                    param("sampleRate", SAMPLE_RATE, &[], ParameterType::Integer),
                    param("channels", CHANNELS, &[], ParameterType::Integer),
                    param("bufferFrames", BUFFER_FRAMES, &[], ParameterType::Integer),
                ],
                return_type: ParameterType::Named(AUDIO_INPUT_TYPE_ID),
                errors: errors(),
                body: super::native_body(&["openInputDevice"]),
            },
        ],
    });
}
