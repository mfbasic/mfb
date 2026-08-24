//! `audio::read` — capture PCM frames from an input stream (native OS-seam).
//!
//! One signature with an optional trailing `timeoutMs` (arity 2..=3); the timed
//! form is selected at codegen (`builder_values` → `audio.readTimeout`), declared
//! here as the code-form alias. `read` is input-only — its `AudioInput` parameter
//! rejects an `AudioOutput` under strict base-resource matching.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, timeout_ms, AUDIO_INPUT_TYPE_ID};

/// `abi_function` body for `audio::read` (and its `readTimeout` timed overload alias)
/// — capture PCM frames. `ctx.call` selects the untimed vs timed form; routes by
/// `platform.family()` to the backend `read` emitter.
pub(crate) fn lower_read(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let timed = ctx.call == "audio.readTimeout";
    let (instructions, relocations, stack_size) = match ctx.platform.family() {
        PlatformFamily::MacOS => {
            super::gen_macos_io::lower_read(&symbol, timed, ctx.platform_imports, ctx.platform)?
        }
        PlatformFamily::Linux => {
            super::gen_alsa_io::lower_read(&symbol, timed, ctx.platform_imports, ctx.platform)?
        }
        PlatformFamily::Windows => {
            super::gen_windows::lower_read(&symbol, timed, ctx.platform_imports, ctx.platform)?
        }
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Capture PCM frames from an input stream as raw `s16le` bytes."#;
const DESC: &str = r#"`audio::read` captures PCM from an open `AudioInput` and returns it as a
`List OF Byte` of raw interleaved `s16le` samples (one frame is `channels * 2`
bytes). `read` is defined only over `AudioInput`; passing an `AudioOutput` is a
compile-time overload-resolution error. The stream is borrowed, not consumed.
`frames` must be in `1..=1048576`. The two-argument form blocks until exactly
`frames` frames are captured. The three-argument form's `timeoutMs` follows the
language timeout convention: a negative value raises `ErrInvalidArgument`; `0`
returns immediately with whatever whole frames are already buffered (a poll); a
positive value waits up to that many milliseconds (clamped to `2147483647`),
returning only whole frames gathered so far — possibly an empty list, never a
partial frame."#;
const EX: &str = r#"Poll for whatever whole frames are already buffered, without blocking:

```
IMPORT audio

SUB main()
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  LET now = audio::read(mic, 4800, 0)
  audio::close(mic)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "read",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("AudioInput, Integer[, Integer]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                param(
                    "input",
                    "An open capture stream, from `audio::openInput`. Borrowed, not consumed. Reading after close raises `ErrAudioDevice`.",
                    &[],
                    ParameterType::named(AUDIO_INPUT_TYPE_ID),
                ),
                param(
                    "frames",
                    "Number of frames to capture. Must be in `1..=1048576`.",
                    &[],
                    ParameterType::Integer,
                ),
                timeout_ms(
                    "Maximum wait in milliseconds (timed overload only). A negative value raises `ErrInvalidArgument`; `0` returns immediately with whatever is buffered; a positive value is clamped to `2147483647`.",
                ),
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrInvalidArgument", "ErrAudioDevice", "ErrAudioUnavailable", "ErrOutOfMemory"],
            body: super::native_body(lower_read, &["readTimeout"]),
        }],
    });
}
