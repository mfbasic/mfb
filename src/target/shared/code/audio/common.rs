//! Platform-neutral scaffolding shared by the CoreAudio (macOS) and ALSA
//! (Linux) audio backends (bug-330): the `openInput`/`openOutput` parameter
//! ranges and their validation emitter, plus the runtime query-kind enum. With
//! no backend type to hang shared code on, `macos.rs` and `alsa.rs` each carried
//! a private copy of all three; they now live here and both backends import
//! them.

use super::*;
use crate::target::shared::abi;

/// The runtime queries a backend answers about a live stream: bytes available,
/// a blocking poll, a timed poll, and the xrun (over/underrun) counter.
#[derive(Clone, Copy)]
pub(super) enum Query {
    Available,
    Poll,
    PollTimeout,
    Xruns,
}

// Parameter validation ranges (plan-33-A §3.5).
const SR_MIN: &str = "8000";
const SR_MAX: &str = "192000";
const BUF_MIN: &str = "64";
const BUF_MAX: &str = "8192";

/// Upper bound on a single `audio::read(frames)` request (plan-33-A §3.5),
/// shared by both backends' read paths.
pub(super) const READ_FRAMES_MAX: &str = "1048576";

/// Validate `openOutput`/`openInput` scalar parameters (sampleRate at `sr_off`,
/// channels at `ch_off`, bufferFrames at `bf_off`, each an sp-relative slot),
/// branching to `invalid` (→ ErrInvalidArgument) on any §3.5 violation.
pub(super) fn emit_validate_open(
    symbol: &str,
    sr_off: usize,
    ch_off: usize,
    bf_off: usize,
    invalid: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let ch_ok = format!("{symbol}_ch_ok");
    instructions.extend([
        // sampleRate in 8000..=192000
        abi::load_u64("%v9", abi::stack_pointer(), sr_off),
        abi::move_immediate("%v10", "Integer", SR_MIN),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_lt(invalid),
        abi::move_immediate("%v10", "Integer", SR_MAX),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_gt(invalid),
        // channels 1 or 2
        abi::load_u64("%v9", abi::stack_pointer(), ch_off),
        abi::compare_immediate("%v9", "1"),
        abi::branch_eq(&ch_ok),
        abi::compare_immediate("%v9", "2"),
        abi::branch_ne(invalid),
        abi::label(&ch_ok),
        // bufferFrames in 64..=8192
        abi::load_u64("%v9", abi::stack_pointer(), bf_off),
        abi::move_immediate("%v10", "Integer", BUF_MIN),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_lt(invalid),
        abi::move_immediate("%v10", "Integer", BUF_MAX),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_gt(invalid),
    ]);
}
