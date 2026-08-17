//! `audio::devices` — enumerate the host's audio devices (native OS-seam).
//!
//! Docs migrated from `src/docs/man/builtins/audio/devices.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::AUDIO_DEVICE_TYPE;

const INTRO: &str = r#"Enumerate the audio devices the operating system reports."#;
const DESC: &str = r#"`audio::devices` takes no arguments and returns every audio device the host
reports, each as an `AudioDevice` record carrying an opaque `id`, a human-readable
`name`, the `canInput`/`canOutput` capability flags, and the
`isDefaultInput`/`isDefaultOutput` flags marking the system defaults. The `id` is a
Core Audio device UID on macOS and an ALSA PCM hint name on Linux; it is opaque —
pass it to `audio::openInput`/`audio::openOutput`, never construct it. On macOS an
empty enumeration raises `ErrAudioUnavailable`; on Linux the list may be empty when
ALSA reports no PCM hints."#;
const EX: &str = r#"List every device and mark its capabilities:

```
IMPORT audio
IMPORT io

SUB main()
  FOR EACH d IN audio::devices()
    io::print(d.name & " in=" & toString(d.canInput) & " out=" & toString(d.canOutput))
  NEXT
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "devices",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::Named(AUDIO_DEVICE_TYPE)),
            errors: vec!["ErrAudioUnavailable", "ErrAudioDevice", "ErrOutOfMemory"],
            body: super::native_body(&[]),
        }],
    });
}
