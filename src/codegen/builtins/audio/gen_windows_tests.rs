// bug-416: emit-inspection regression guards for the Windows WASAPI backend.
// Runtime proof of every WASAPI path is Windows-only (box 2230); these lower the
// private helpers with the shared `TestPlatform` and pin the neutral instruction
// stream so the four batched defects cannot silently regress.
// --- codegen tier imports (migration) ---
use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::codegen::engine::tests::TestPlatform;
use std::collections::HashMap;
/// Count of `emit_read_fill` expansions in an instruction stream: each emits one
/// unique `rfill_direct_<n>` label (the EXCLUSIVE branch target).
fn read_fill_count(ins: &[CodeInstruction]) -> usize {
    ins.iter()
        .filter(|i| {
            i.op == CodeOp::Label
                && i.get("name")
                    .is_some_and(|name| name.starts_with("rfill_direct_"))
        })
        .count()
}

fn any_label_with(ins: &[CodeInstruction], needle: &str) -> bool {
    ins.iter()
        .any(|i| i.op == CodeOp::Label && i.get("name").is_some_and(|name| name.contains(needle)))
}

/// bug-416 (1): `audio::available` returns whole FRAMES, not bytes. The
/// `Query::Available` arm must NOT scale the frame count by bytes-per-frame —
/// i.e. it emits no `mul` writing the result value register.
#[test]
fn available_returns_frames_not_bytes() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) = lower_query("t_avail", Query::Available, &imports, &TestPlatform)
        .expect("lower audio::available");
    let scales_result = ins.iter().any(|i| {
        i.op == CodeOp::Mul
            && i.get("dst").as_deref() == Some(RESULT_VALUE_REGISTER.to_string().as_str())
    });
    assert!(
        !scales_result,
        "bug-416 (1): audio::available must return frames, not frames*bpf \
         (found a `mul` writing the result register)"
    );
}

/// bug-416 (2): a non-packet-aligned `audio::read` must lose no captured frames.
/// The fix drains a carry-over of the previous read's unconsumed tail (a second
/// `emit_read_fill`) and saves the current packet's unconsumed tail before
/// `ReleaseBuffer` (a distinctly-labelled guard).
#[test]
fn read_preserves_unconsumed_capture_tail() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) =
        lower_read("t_read", false, &imports, &TestPlatform).expect("lower audio::read");
    // Drain of the previous read's carry-over is a second frame-fill expansion.
    assert_eq!(
        read_fill_count(&ins),
        2,
        "bug-416 (2): audio::read must drain a carry-over tail (a second emit_read_fill) \
         in addition to the per-packet fill"
    );
    // The current packet's unconsumed tail is stashed before ReleaseBuffer.
    assert!(
        any_label_with(&ins, "carry_tail"),
        "bug-416 (2): audio::read must save the unconsumed capture tail before ReleaseBuffer"
    );
}

/// bug-416 (2), readTimeout shares the same helper and must carry too.
#[test]
fn read_timeout_preserves_unconsumed_capture_tail() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) =
        lower_read("t_readto", true, &imports, &TestPlatform).expect("lower audio::readTimeout");
    assert_eq!(
        read_fill_count(&ins),
        2,
        "bug-416 (2): readTimeout must drain a carry-over tail"
    );
    assert!(
        any_label_with(&ins, "carry_tail"),
        "bug-416 (2): readTimeout must save the unconsumed capture tail before ReleaseBuffer"
    );
}

/// bug-416 (3): the SHARED-mix open must reject `mixCh < userCh` (else the read
/// converter reads 4 bytes past the capture buffer). The guard reads the stored
/// mix channel count (`W_MIX_CH`) to compare it against the user channel count;
/// before the fix `lower_open` only STORES `W_MIX_CH`, never loads it.
#[test]
fn shared_open_guards_mix_channel_underflow() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) = lower_open("t_openin", true, false, &imports, &TestPlatform)
        .expect("lower audio::openInput");
    let mix_ch = W_MIX_CH.to_string();
    let loads_mix_ch = ins.iter().any(|i| {
        matches!(i.op, CodeOp::LdrU64 | CodeOp::LdrU32 | CodeOp::LdrU16)
            && i.get("offset").as_deref() == Some(mix_ch.as_str())
    });
    assert!(
        loads_mix_ch,
        "bug-416 (3): openInput must load W_MIX_CH to reject mixCh < userCh"
    );
}

/// plan-132 C4: a device-specific open reads the `id` of the `audio::AudioDevice`
/// record at `DEVID_OFF` as the `String` inlined at
/// `record + [record + DEVICE_FIELD_ID]`. The widen used to take the record pointer
/// itself as the `String` — its first word read as the length, the next as bytes — so
/// `openOutputDevice`/`openInputDevice` handed WASAPI a garbage endpoint id. No host
/// running this suite has an audio device, so the rebase is pinned on the emitted
/// stream: load the record, load its slot-0 offset from it, add the two.
#[test]
fn widen_device_id_reads_the_id_inlined_in_the_device_record() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let mut ins = Vec::new();
    let mut vregs = Vregs::new();
    emit_widen_device_id(&mut ins, &mut vregs);

    assert_eq!(
        ins[0].op,
        CodeOp::LdrU64,
        "the widen starts by loading the record"
    );
    assert_eq!(ins[0].get("base").as_deref(), Some(abi::SP_SPELLING));
    assert_eq!(
        ins[0].get("offset").as_deref(),
        Some(DEVID_OFF.to_string().as_str())
    );
    let record = ins[0].get("dst");
    assert!(record.is_some());

    assert_eq!(
        ins[1].op,
        CodeOp::LdrU64,
        "then the id's block-relative offset"
    );
    assert_eq!(ins[1].get("base").as_deref(), record.as_deref());
    assert_eq!(
        ins[1].get("offset").as_deref(),
        Some(DEVICE_FIELD_ID.to_string().as_str())
    );
    let offset = ins[1].get("dst");

    assert_eq!(
        ins[2].op,
        CodeOp::Add,
        "then record + offset is the id String"
    );
    assert_eq!(ins[2].get("lhs").as_deref(), record.as_deref());
    assert_eq!(ins[2].get("rhs").as_deref(), offset.as_deref());
}

use crate::codegen::builtins::audio::status_tests::{raises_of, set};

/// plan-158-C: `audio::write` never waits without a bound. `WaitForSingleObject`'s
/// `INFINITE` is `!0`, emitted as `mvn` into its timeout argument; a write that
/// cannot progress (an invalidated device, a stalled engine) must fail instead.
#[test]
fn write_has_no_infinite_wait() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _) = lower_write("t_write", &imports, &TestPlatform).expect("write");
    let timeout_arg = abi::c_arg(1).to_string();
    assert!(
        !ins.iter()
            .any(|i| i.op == CodeOp::Mvn && i.get("dst").as_deref() == Some(timeout_arg.as_str())),
        "plan-158-C: audio::write must bound every WaitForSingleObject"
    );
}

/// plan-158-C: an EXCLUSIVE output is pre-rolled before it starts, so its open
/// skips `Start` (and only `Start`: one indirect call between the skip and its
/// target) when `W_SHARED` is 0. An input stream always starts at open.
#[test]
fn exclusive_output_open_does_not_start_the_stream() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _) = lower_open("t_open", false, false, &imports, &TestPlatform).expect("open");
    let label = ins
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("t_open_opened"))
        .expect("the open's post-Start label");
    let skip = ins
        .iter()
        .position(|i| i.op == CodeOp::BranchEq && i.get("target").as_deref() == Some("t_open_opened"))
        .expect("the EXCLUSIVE output skips Start");
    assert!(skip < label);
    let calls = ins[skip..label]
        .iter()
        .filter(|i| i.op == CodeOp::BranchLinkRegister)
        .count();
    assert_eq!(calls, 1, "exactly the Start call is skipped");
    let (ins, _rel, _) = lower_open("t_open", true, false, &imports, &TestPlatform).expect("open");
    assert!(
        !ins.iter()
            .any(|i| i.op == CodeOp::BranchEq && i.get("target").as_deref() == Some("t_open_opened")),
        "an input stream always starts at open"
    );
}

/// plan-158-A: every Windows audio failure that has an OS status reports it. No
/// `ErrAudioDevice` / `ErrAudioUnavailable` in these helpers is raised with the
/// bare static message except the closed-stream refusals of `write`/`read`, which
/// have no OS call behind them; every other one goes through
/// `_mfb_rt_fail_status` under the call name listed here.
#[test]
fn windows_audio_failures_report_their_os_status() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    // plan-158-C: SHARED first, EXCLUSIVE only when the mix format cannot serve the
    // request, so the mix checks choose the mode instead of failing, and each
    // `Initialize` names the mode it tried.
    let open_calls = [
        "IMMDevice::Activate",
        "IAudioClient::GetMixFormat",
        "IAudioClient::Initialize(SHARED)",
        "IAudioClient::Initialize(EXCLUSIVE)",
        "IAudioClient::GetBufferSize",
        "CreateEventW",
        "IAudioClient::SetEventHandle",
        "IAudioClient::GetService",
        "IAudioClient::Start",
    ];
    for (input, device) in [(false, false), (true, false), (false, true), (true, true)] {
        let (ins, rel, _) = lower_open("t_open", input, device, &imports, &TestPlatform)
            .expect("lower audio::open*");
        let mut expected = set(&open_calls);
        expected.insert(
            if device {
                "IMMDeviceEnumerator::GetDevice"
            } else {
                "IMMDeviceEnumerator::GetDefaultAudioEndpoint"
            }
            .to_string(),
        );
        assert_eq!(
            raises_of(&ins, &rel, "ErrAudioDevice"),
            (0, expected),
            "open input={input} device={device}"
        );
        assert_eq!(
            raises_of(&ins, &rel, "ErrAudioUnavailable"),
            (0, set(&["CoCreateInstance"])),
            "open input={input} device={device}"
        );
    }
    let (ins, rel, _) = lower_write("t_write", &imports, &TestPlatform).expect("lower audio::write");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioDevice"),
        (
            1,
            set(&[
                "WaitForSingleObject",
                "IAudioClient::GetCurrentPadding",
                "IAudioRenderClient::GetBuffer",
                "IAudioRenderClient::ReleaseBuffer",
                // plan-158-C: the EXCLUSIVE submit starts the pre-rolled stream.
                "IAudioClient::Start",
            ])
        )
    );
    for timeout in [false, true] {
        let (ins, rel, _) =
            lower_read("t_read", timeout, &imports, &TestPlatform).expect("lower audio::read");
        assert_eq!(
            raises_of(&ins, &rel, "ErrAudioDevice"),
            (
                1,
                set(&[
                    "IAudioCaptureClient::GetBuffer",
                    "IAudioCaptureClient::ReleaseBuffer",
                ])
            ),
            "read timeout={timeout}"
        );
    }
    let (ins, rel, _) =
        lower_devices("t_devices", &imports, &TestPlatform).expect("lower audio::devices");
    assert_eq!(
        raises_of(&ins, &rel, "ErrAudioUnavailable"),
        (
            0,
            set(&[
                "CoCreateInstance",
                "IMMDeviceEnumerator::EnumAudioEndpoints",
                "IMMDeviceCollection::GetCount",
                "IMMDeviceCollection::Item",
            ])
        )
    );
}
