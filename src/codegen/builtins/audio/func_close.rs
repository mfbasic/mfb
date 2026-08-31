//! `audio::close` — close a stream and release its OS resources (native OS-seam).
//!
//! `close` spans both handle types: `close(AudioInput)` and `close(AudioOutput)` —
//! two overloads over the resource pair, both returning `Nothing`, both **consuming**
//! the handle. The public name always lowers per direction to the internal
//! `audio.closeInput` / `audio.closeOutput` bodies (the teardown sequences differ);
//! each is declared as its overload's code-form `os_alias` and is that direction's
//! registered resource close op (so scope-drop reaches it directly). Docs migrated
//! from `src/docs/man/builtins/audio/close.md`.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID};

/// `abi_function` body for `audio::close` — close a stream. The member is only ever
/// invoked as one of its per-direction code forms (`closeInput`/`closeOutput`, the
/// IR-level overload split); `ctx.call` selects the direction. macOS has
/// direction-specific close emitters; ALSA and WASAPI take a unified
/// `lower_close(is_input)`.
pub(crate) fn lower_close(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let is_input = ctx.call == super::CLOSE_INPUT;
    let (instructions, relocations, stack_size) = match ctx.platform.family() {
        PlatformFamily::MacOS => {
            if is_input {
                super::gen_macos_stream::lower_close_input(
                    &symbol,
                    ctx.platform_imports,
                    ctx.platform,
                )?
            } else {
                super::gen_macos_stream::lower_close_output(
                    &symbol,
                    ctx.platform_imports,
                    ctx.platform,
                )?
            }
        }
        PlatformFamily::Linux => super::gen_alsa_stream::lower_close(
            &symbol,
            is_input,
            ctx.platform_imports,
            ctx.platform,
        )?,
        PlatformFamily::Windows => {
            super::gen_windows::lower_close(&symbol, is_input, ctx.platform_imports, ctx.platform)?
        }
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Close an audio stream and release its operating-system resources; the handle cannot be used again."#;
const DESC: &str = r#"`audio::close` shuts an open capture or playback stream down and releases the
underlying OS objects, returning `Nothing`. It accepts either direction, and
does the right thing for each — a capture stream and a playback stream shut down
differently. Unlike every other `audio::` call, `close` ends its stream handle: it cannot be
used afterward. Closing an `AudioOutput` first
drains queued playback (so it can block until the audio already written has finished
sounding); closing an `AudioInput` drops any buffered capture immediately. `close` is
idempotent — closing an already-closed or defaulted handle is a no-op that returns
successfully. A stream also closes itself when its binding goes out of scope."#;
const EX: &str = r#"Close an output stream explicitly after playback:

```
IMPORT audio

SUB main()
  RES out AS audio::AudioOutput = audio::openOutput(48000, 2, 512)
  LET pcm AS List OF Byte = [0, 0, 0, 0]
  audio::write(out, pcm)
  audio::close(out)
END SUB
```"#;

const STREAM_DESC: &str = "An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Closed by this call; the handle cannot be used again. A closed handle is a no-op.";

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "close",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("AudioInput or AudioOutput"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![param(
                    "stream",
                    STREAM_DESC,
                    &[],
                    ParameterType::named(AUDIO_INPUT_TYPE_ID),
                )],
                return_type: ParameterType::Nothing,
                errors: vec!["ErrAudioUnavailable"],
                body: super::native_body(lower_close, &["closeInput"]),
            },
            Implementation {
                params: vec![param(
                    "stream",
                    STREAM_DESC,
                    &[],
                    ParameterType::named(AUDIO_OUTPUT_TYPE_ID),
                )],
                return_type: ParameterType::Nothing,
                errors: vec!["ErrAudioUnavailable"],
                body: super::native_body(lower_close, &["closeOutput"]),
            },
        ],
    });
}
