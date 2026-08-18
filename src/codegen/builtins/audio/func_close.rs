//! `audio::close` — close a stream and release its OS resources (native OS-seam).
//!
//! `close` spans both handle types: `close(AudioInput)` and `close(AudioOutput)` —
//! two overloads over the resource pair, both returning `Nothing`, both **consuming**
//! the handle. The public name always lowers per direction to the internal
//! `audio.closeInput` / `audio.closeOutput` bodies (the teardown sequences differ);
//! each is declared as its overload's code-form `os_alias` and is that direction's
//! registered resource close op (so scope-drop reaches it directly). Docs migrated
//! from `src/docs/man/builtins/audio/close.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID};

const INTRO: &str =
    r#"Close an audio stream and release its operating-system resources, consuming the handle."#;
const DESC: &str = r#"`audio::close` shuts an open capture or playback stream down and releases the
underlying OS objects, returning `Nothing`. It is defined over both directions;
IR lowering routes each operand to a distinct per-direction internal body
(`audio.closeInput` / `audio.closeOutput`), because their teardown sequences differ.
Unlike every other `audio::` call, `close` consumes its stream handle: the binding is
moved into the call and cannot be used afterward. Closing an `AudioOutput` first
drains queued playback (so it can block until the audio already written has finished
sounding); closing an `AudioInput` drops any buffered capture immediately. `close` is
idempotent — closing an already-closed or defaulted handle is a no-op that returns
successfully. A stream is also closed automatically by lexical drop when its binding
leaves scope."#;
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

const STREAM_DESC: &str = "An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Consumed by the call — the handle is moved and unusable afterward. A closed handle is a no-op.";

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
                    ParameterType::Named(AUDIO_INPUT_TYPE_ID),
                )],
                return_type: ParameterType::Nothing,
                errors: vec!["ErrAudioUnavailable"],
                body: super::native_body(&["closeInput"]),
            },
            Implementation {
                params: vec![param(
                    "stream",
                    STREAM_DESC,
                    &[],
                    ParameterType::Named(AUDIO_OUTPUT_TYPE_ID),
                )],
                return_type: ParameterType::Nothing,
                errors: vec!["ErrAudioUnavailable"],
                body: super::native_body(&["closeOutput"]),
            },
        ],
    });
}
