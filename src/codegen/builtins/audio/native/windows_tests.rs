// bug-416: emit-inspection regression guards for the Windows WASAPI backend.
// Runtime proof of every WASAPI path is Windows-only (box 2230); these lower the
// private helpers with the shared `TestPlatform` and pin the neutral instruction
// stream so the four batched defects cannot silently regress.
// --- codegen tier imports (migration) ---
use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::app::hook as app;
use crate::codegen::app::hook::*;
use crate::codegen::cleanup::owned::*;
use crate::codegen::cleanup::thread::*;
use crate::codegen::collection::assign::*;
use crate::codegen::collection::buffer::*;
use crate::codegen::collection::compare::*;
use crate::codegen::collection::layout::*;
use crate::codegen::collection::list::*;
use crate::codegen::collection::map::*;
use crate::codegen::collection::search::*;
use crate::codegen::collection::sort::*;
use crate::codegen::compiler::opt::*;
use crate::codegen::engine::analysis::*;
use crate::codegen::engine::arch::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::control::*;
use crate::codegen::engine::convert::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::operators::*;
use crate::codegen::engine::tests::TestPlatform;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::engine::validation;
use crate::codegen::engine::validation::*;
use crate::codegen::engine::value::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::error::result::*;
use crate::codegen::io::stdin::*;
use crate::codegen::io::stdout::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::arena::*;
use crate::codegen::memory::data::*;
use crate::codegen::memory::marshal::*;
use crate::codegen::memory::value::*;
use crate::codegen::os::ffi::*;
use crate::codegen::os::process::*;
use crate::codegen::os::syscall::*;
use crate::codegen::resource::cleanup::*;
use crate::codegen::runtime::thread::*;
use crate::codegen::string::format::*;
use crate::codegen::string::repr::*;
use crate::codegen::string::util::*;
use crate::codegen::string::validate::*;
use crate::codegen::term::core as term;
use crate::codegen::term::core::*;
use crate::codegen::term::grid::*;
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
    let (_frame, ins, _rel, _slots) =
        lower_query("t_avail", Query::Available, &imports, &TestPlatform)
            .expect("lower audio::available");
    let scales_result = ins.iter().any(|i| {
        i.op == CodeOp::Mul
            && i.get("dst").as_deref() == Some(RESULT_VALUE_REGISTER.render().as_str())
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
    let (_frame, ins, _rel, _slots) =
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
    let (_frame, ins, _rel, _slots) =
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
    let (_frame, ins, _rel, _slots) = lower_open("t_openin", true, false, &imports, &TestPlatform)
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
