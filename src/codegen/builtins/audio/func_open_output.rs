//! `audio::openOutput` — open a playback stream (native OS-seam).
//!
//! Two overloads (default-device / named-device), the named form declaring the
//! code-form alias `openOutputDevice`.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_DEVICE_TYPE, AUDIO_OUTPUT_TYPE_ID};

/// `abi_function` body for `audio::openOutput` (and its `openOutputDevice`
/// named-device overload alias) — open a playback stream. `ctx.call` selects the
/// default-device vs named-device form; the shared [`super::gen_shared::dispatch_open`]
/// routes by `platform.family()` to the backend `open` emitter.
pub(crate) fn lower_open_output(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let device = ctx.call == "audio.openOutputDevice";
    let (instructions, relocations, stack_size) = super::gen_shared::dispatch_open(
        &symbol,
        false,
        device,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Open a playback stream and return an `AudioOutput` handle."#;
const DESC: &str = r#"`audio::openOutput` opens a PCM playback stream and returns an `AudioOutput`. The
three-argument form opens the system default output device; the four-argument form
opens the specific device named by an `AudioDevice` from `audio::devices()`. The
stream carries raw interleaved `s16le` PCM (one frame is `channels * 2` bytes).
`sampleRate` must be in `8000..=192000`, `channels` must be `1` or `2`, and
`bufferFrames` must be in `64..=8192`; any value outside these bounds raises
`ErrInvalidArgument`. `channels`/`sampleRate` are not resampled: on Linux the
committed rate and channel count must match the request exactly or the call raises
`ErrAudioDevice`. The returned `AudioOutput` is a move-only, non-sendable resource
closed by lexical drop or `audio::close`; feed it with `audio::write` or
`audio::play`, both defined only over `AudioOutput`."#;
const EX: &str = r#"Open the default mono output at 48 kHz and play a short MML tune:

```
IMPORT audio

SUB main()
  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::play(out, "T120 O4 L8 I sine C E G")
  audio::close(out)
END SUB
```"#;

const SAMPLE_RATE: &str = "Playback rate in Hz. Must be in `8000..=192000`.";
const CHANNELS: &str = "Channel count: `1` (mono) or `2` (stereo).";
const BUFFER_FRAMES: &str =
    "Frames per OS buffer. Must be in `64..=8192`; need not be a power of two.";
const DEVICE: &str = "The device to open, from `audio::devices()` with `canOutput` set (four-argument form only). A device whose `id` no longer exists raises `ErrAudioDevice`.";

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let errors = || {
        vec![
            "ErrInvalidArgument",
            "ErrAudioUnavailable",
            "ErrAudioDevice",
            "ErrOutOfMemory",
        ]
    };
    pkg.add_function(RegistryFunction {
        name: "openOutput",
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
                return_type: ParameterType::named(AUDIO_OUTPUT_TYPE_ID),
                errors: errors(),
                body: super::native_body(lower_open_output, &[]),
            },
            Implementation {
                params: vec![
                    param(
                        "device",
                        DEVICE,
                        &[],
                        ParameterType::named(AUDIO_DEVICE_TYPE),
                    ),
                    param("sampleRate", SAMPLE_RATE, &[], ParameterType::Integer),
                    param("channels", CHANNELS, &[], ParameterType::Integer),
                    param("bufferFrames", BUFFER_FRAMES, &[], ParameterType::Integer),
                ],
                return_type: ParameterType::named(AUDIO_OUTPUT_TYPE_ID),
                errors: errors(),
                body: super::native_body(lower_open_output, &["openOutputDevice"]),
            },
        ],
    });
}
