//! `audio::xruns` — cumulative overrun/underrun count for a stream (native OS-seam).
//!
//! Two overloads (`AudioInput` / `AudioOutput`) sharing one runtime symbol
//! (`audio.xruns`, branching on the handle kind at runtime).

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_common::Query;
use super::{param, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID};

/// `abi_function` body for `audio::xruns` — the cumulative over/underrun count.
/// Routes via the shared [`super::gen_shared::dispatch_query`] with [`Query::Xruns`]
/// (no overload aliases).
pub(crate) fn lower_xruns(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::dispatch_query(
        &symbol,
        Query::Xruns,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

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
            ParameterType::named(stream_ty),
        )],
        return_type: ParameterType::Integer,
        errors: vec![],
        body: super::native_body(lower_xruns, &[]),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
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
