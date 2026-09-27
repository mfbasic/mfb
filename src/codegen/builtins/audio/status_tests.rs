//! plan-158-A/-B: emit-inspection of the audio backends' failure raises. A native
//! helper's OS-status failure must go through `_mfb_rt_fail_status` under its call
//! name; only a failure with no OS call behind it keeps the bare static message.
//! No host running this suite has every backend's audio stack, so the raises are
//! pinned on the lowered instruction stream.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::codegen::engine::tests::TestPlatform;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use std::collections::{BTreeSet, HashMap};

/// What a lowered helper raises. `static_raises` counts the fixed-message raises of
/// `error_name` (`raise_error_into`: the message address loaded straight into the
/// error-message register); `status_calls` lists the call names of the
/// `_mfb_rt_fail_status` sites, decoded from their message-prefix symbols.
pub(crate) fn raises_of(
    ins: &[CodeInstruction],
    rel: &[CodeRelocation],
    error_name: &str,
) -> (usize, BTreeSet<String>) {
    let (_, message_symbol) =
        crate::codegen::registry::runtime_error_emission(error_name).expect("known error");
    let message_register = RESULT_ERROR_MESSAGE_REGISTER.to_string();
    let static_raises = ins
        .iter()
        .filter(|i| {
            i.op == CodeOp::Adrp
                && i.get("symbol").as_deref() == Some(message_symbol)
                && i.get("dst").as_deref() == Some(message_register.as_str())
        })
        .count();
    let prefix = format!("{ERROR_STATUS_PREFIX_SYMBOL_PREFIX}{error_name}_");
    let status_calls = rel
        .iter()
        .filter(|r| r.kind == RelocIntent::DataAddrHi && r.to.starts_with(&prefix))
        .map(|r| {
            let text = crate::codegen::memory::data::decode_error_status_prefix_symbol(&r.to)
                .expect("well-formed prefix");
            let call = text.rsplit_once(" (").expect("prefix shape").1;
            call.trim_end_matches(": ").to_string()
        })
        .collect();
    (static_raises, status_calls)
}

pub(crate) fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// plan-158-B: Core Audio. The static `ErrAudioDevice` raises left are the
/// closed-stream refusals and the opens' `dev_fail` tail (a failed `mmap`, a NULL
/// `CFStringCreateWithCString`); `devices`' is its failed `CFStringGetCString`.
#[test]
fn macos_audio_failures_report_their_os_status() {
    use super::gen_macos_devices::lower_devices;
    use super::gen_macos_io::{lower_read, lower_write};
    use super::gen_macos_stream::{lower_open_input, lower_open_output};
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    for device in [false, true] {
        let (ins, rel, _) =
            lower_open_output("t_out", device, &imports, &TestPlatform).expect("openOutput");
        let mut expected = set(&[
            "AudioQueueNewOutput",
            "AudioQueueAllocateBuffer",
            "AudioQueueStart",
        ]);
        if device {
            expected.insert("AudioQueueSetProperty".to_string());
        }
        assert_eq!(raises_of(&ins, &rel, "ErrAudioDevice"), (1, expected), "device={device}");

        let (ins, rel, _) =
            lower_open_input("t_in", device, &imports, &TestPlatform).expect("openInput");
        let mut expected = set(&[
            "AudioQueueNewInput",
            "AudioQueueAllocateBuffer",
            "AudioQueueEnqueueBuffer",
            "AudioQueueStart",
        ]);
        if device {
            expected.insert("AudioQueueSetProperty".to_string());
        }
        assert_eq!(raises_of(&ins, &rel, "ErrAudioDevice"), (1, expected), "device={device}");
    }
    let (ins, rel, _) = lower_open_input("t_in", false, &imports, &TestPlatform).expect("openInput");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioUnavailable"),
        (1, set(&["AudioObjectGetPropertyData"]))
    );
    let (ins, rel, _) = lower_write("t_write", &imports, &TestPlatform).expect("write");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioDevice"),
        (1, set(&["AudioQueueEnqueueBuffer"]))
    );
    let (ins, rel, _) = lower_read("t_read", false, &imports, &TestPlatform).expect("read");
    assert_eq!(raises_of(&ins, &rel, "ErrAudioDevice"), (1, set(&[])));
    let (ins, rel, _) = lower_devices("t_devices", &imports, &TestPlatform).expect("devices");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioDevice"),
        (1, set(&["AudioObjectGetPropertyData"]))
    );
}

/// plan-158-B: ALSA. The static raises left are the closed-stream refusals, the
/// open's failed `mmap` (`ErrAudioDevice`), and a missing `libasound` or symbol
/// (`ErrAudioUnavailable`).
#[test]
fn alsa_audio_failures_report_their_os_status() {
    use super::gen_alsa_devices::lower_devices;
    use super::gen_alsa_io::{lower_read, lower_write};
    use super::gen_alsa_stream::lower_open;
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    for (input, device) in [(false, false), (true, false), (false, true), (true, true)] {
        let (ins, rel, _) =
            lower_open("t_open", input, device, &imports, &TestPlatform).expect("open");
        assert_eq!(
            raises_of(&ins, &rel, "ErrAudioDevice"),
            (
                1,
                set(&[
                    "snd_pcm_open",
                    "snd_pcm_hw_params_malloc",
                    "snd_pcm_hw_params_any",
                    "snd_pcm_hw_params_set_access",
                    "snd_pcm_hw_params_set_format",
                    "snd_pcm_hw_params_set_channels",
                    "snd_pcm_hw_params_set_rate_near",
                    "snd_pcm_hw_params_set_period_size_near",
                    "snd_pcm_hw_params_set_buffer_size_near",
                    "snd_pcm_hw_params",
                    "rate",
                    "channels",
                    "snd_pcm_prepare",
                ])
            ),
            "input={input} device={device}"
        );
        assert_eq!(
            raises_of(&ins, &rel, "ErrAudioUnavailable"),
            (1, set(&[])),
            "input={input} device={device}"
        );
    }
    let (ins, rel, _) = lower_write("t_write", &imports, &TestPlatform).expect("write");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioDevice"),
        (1, set(&["snd_pcm_recover"]))
    );
    for timeout in [false, true] {
        let (ins, rel, _) =
            lower_read("t_read", timeout, &imports, &TestPlatform).expect("read");
        assert_eq!(
            raises_of(&ins, &rel, "ErrAudioDevice"),
            (1, set(&["snd_pcm_recover"])),
            "timeout={timeout}"
        );
    }
    let (ins, rel, _) = lower_devices("t_devices", &imports, &TestPlatform).expect("devices");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioUnavailable"),
        (1, set(&["snd_device_name_hint"]))
    );
}
