//! `audio::write` — queue raw `s16le` PCM to an output stream (native OS-seam).
//!
//! Output-only: its `AudioOutput` parameter rejects an `AudioInput` under strict
//! base-resource matching.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_OUTPUT_TYPE_ID};

/// `abi_function` body for `audio::write` — play PCM frames to an output stream.
/// Routes by `platform.family()` to the backend `write` emitter (no overload aliases).
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = match ctx.platform.family() {
        PlatformFamily::MacOS => {
            super::gen_macos_io::lower_write(&symbol, ctx.platform_imports, ctx.platform)?
        }
        PlatformFamily::Linux => {
            super::gen_alsa_io::lower_write(&symbol, ctx.platform_imports, ctx.platform)?
        }
        PlatformFamily::Windows => {
            super::gen_windows::lower_write(&symbol, ctx.platform_imports, ctx.platform)?
        }
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str =
    r#"Queue raw `s16le` PCM to an output stream, blocking until every byte is enqueued."#;
const DESC: &str = r#"`audio::write` queues raw interleaved `s16le` PCM for playback on an open
`AudioOutput` and blocks until every byte has been handed to the operating system.
It returns `Nothing`. `write` is defined only over `AudioOutput`; passing an
`AudioInput` is a compile-time overload-resolution error. The stream is borrowed,
not consumed. `bytes` must be nonzero in length and an exact whole number of frames
(a multiple of the stream's `channels * 2` bytes-per-frame); a zero-length or
non-frame-aligned list raises `ErrInvalidArgument`. On macOS a tail too short to
fill one buffer is carried in the stream and completed by the next `write` or padded
with silence by `audio::close`; on Linux an underrun bumps the `audio::xruns`
counter and recovers rather than aborting."#;
const EX: &str = r#"Open a stereo output at 48 kHz and play a buffer of PCM:

```
IMPORT audio

SUB main()
  RES out AS audio::AudioOutput = audio::openOutput(48000, 2, 512)
  LET pcm AS List OF Byte = [0, 0, 0, 0]
  audio::write(out, pcm)
  audio::close(out)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // Hand-authored so the argument-mismatch diagnostic names the resource by its
        // bare handle type (`AudioOutput`), not its package-qualified id
        // (`audio.AudioOutput`) — matching the pre-migration message and every other
        // audio member's phrasing.
        expected_arguments: Some("AudioOutput, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                param(
                    "output",
                    "An open playback stream, from `audio::openOutput`. Borrowed, not consumed. Writing after close raises `ErrAudioDevice`.",
                    &[],
                    ParameterType::Named(AUDIO_OUTPUT_TYPE_ID),
                ),
                param(
                    "bytes",
                    "Interleaved `s16le` PCM. Length must be nonzero and a whole multiple of `channels * 2` (one frame).",
                    &[],
                    ParameterType::list_of(ParameterType::Byte),
                ),
            ],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrInvalidArgument", "ErrAudioDevice", "ErrAudioUnavailable"],
            body: super::native_body(lower_write, &[]),
        }],
    });
}
