//! `audio::poll` — test a stream for readiness (native OS-seam).
//!
//! Two overloads (`AudioInput` / `AudioOutput`), each with an optional trailing
//! `timeoutMs` (arity 1..=2). Both directions share one runtime symbol
//! (`audio.poll`, branching on the handle kind at runtime); the timed form is
//! selected at codegen (`builder_values` → `audio.pollTimeout`), declared here as
//! the code-form alias. Docs migrated from `src/docs/man/builtins/audio/poll.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::{param, timeout_ms, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID};

const INTRO: &str = r#"Test an open stream for readiness, optionally waiting up to a deadline."#;
const DESC: &str = r#"`audio::poll` reports whether an open stream is ready for its next I/O operation,
returning a `Boolean`. For an `AudioInput`, ready means at least one whole frame can
be read; for an `AudioOutput`, ready means at least one buffer is free to write.
`poll` is defined over both directions, and the untimed form is exactly
`audio::available(stream) > 0`. It follows the language timeout convention: the
one-argument form blocks until the stream is ready, then returns `TRUE`; the
two-argument form waits up to `timeoutMs` milliseconds, returning `TRUE` the moment
the stream is ready and `FALSE` at the deadline. A `timeoutMs` of `0` is a
non-blocking test; a negative value raises `ErrInvalidArgument`; a positive value is
clamped to `2147483647`. A closed or defaulted handle polls as `FALSE` rather than
raising."#;
const EX: &str = r#"Drive a capture stream only when at least one frame is ready, waiting up to 50 ms:

```
IMPORT audio

SUB main()
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  IF audio::poll(mic, 50) THEN
    LET pcm = audio::read(mic, 480, 0)
  END IF
  audio::close(mic)
END SUB
```"#;

const STREAM_DESC: &str = "An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Borrowed, not consumed. A closed handle polls as `FALSE`.";
const TIMEOUT_DESC: &str = "Maximum wait in milliseconds (timed overload only). `0` is a non-blocking test; a negative value raises `ErrInvalidArgument`; a positive value is clamped to `2147483647`.";

fn overload(stream_ty: &'static str) -> Implementation {
    Implementation {
        params: vec![
            param("stream", STREAM_DESC, &[], ParameterType::Named(stream_ty)),
            timeout_ms(TIMEOUT_DESC),
        ],
        return_type: ParameterType::Boolean,
        errors: vec!["ErrInvalidArgument", "ErrAudioUnavailable"],
        body: super::native_body(&["pollTimeout"]),
    }
}

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("AudioInput or AudioOutput[, Integer]"),
        internal_only: false,
        implementations: vec![
            overload(AUDIO_INPUT_TYPE_ID),
            overload(AUDIO_OUTPUT_TYPE_ID),
        ],
    });
}
