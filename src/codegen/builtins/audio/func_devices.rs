//! `audio::devices` — enumerate the host's audio devices (native OS-seam).

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::AUDIO_DEVICE_TYPE;

/// `abi_function` body for `audio::devices` — enumerate audio devices. Routes by
/// `platform.family()` to the backend device enumerator (no overload aliases).
pub(crate) fn lower_devices(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = match ctx.platform.family() {
        PlatformFamily::MacOS => {
            super::gen_macos_devices::lower_devices(&symbol, ctx.platform_imports, ctx.platform)?
        }
        PlatformFamily::Linux => {
            super::gen_alsa_devices::lower_devices(&symbol, ctx.platform_imports, ctx.platform)?
        }
        PlatformFamily::Windows => {
            super::gen_windows::lower_devices(&symbol, ctx.platform_imports, ctx.platform)?
        }
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Enumerate the audio devices the operating system reports."#;
const DESC: &str = r#"`audio::devices` takes no arguments and returns every audio device the host
reports, each as an `audio::AudioDevice` record carrying an opaque `id`, a human-readable
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "devices",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::named(AUDIO_DEVICE_TYPE)),
            errors: vec!["ErrAudioUnavailable", "ErrAudioDevice", "ErrOutOfMemory"],
            body: super::native_body(lower_devices, &[]),
        }],
    });
}
