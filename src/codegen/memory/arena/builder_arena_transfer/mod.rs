// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::result::UnboxedPayloadSource;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

mod collection_transfer;
mod result;
mod transfer;

/// bug-566: who owns the block that the RAW success value points at, on the `Ok`
/// path of [`CodeBuilder::materialize_current_result`].
///
/// The lowering copies that block twice — once into an intermediate
/// (`copy_value_to_current_arena`), then out of the intermediate INTO the flat
/// `Result` (`emit_build_result_inline`) — and frees the intermediate (bug-379).
/// The producer's ORIGINAL is dead from that moment. Whether this frame may free
/// it is not a property of the `Result` lowering at all; it is a property of the
/// PRODUCER, so every one of the six call sites states it, and there is no
/// default. Answering `OwnedByThisFrame` wrongly is a double free or, for a
/// worker's block, a cross-arena wild free — `x19` is per-thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RawSuccessBlock {
    /// This frame allocated it (through a callee that allocated in THIS thread's
    /// arena and handed the only pointer back), and no other owner will free it.
    /// Free it after the copy.
    OwnedByThisFrame,
    /// Someone else frees it, or it is not this arena's to free at all. Emit
    /// nothing.
    OwnedElsewhere,
    /// bug-648: a resource (or resource union) ELEMENT another owner still holds
    /// and closes — `tcp`/`udp`/`tls::poll` over a list, `collections::get`/`getOr`.
    /// Carry the pointer into the `Result`: a copy would run the thread hand-over
    /// lowering, which tombstones the live element `moved|closed`. Free nothing.
    /// bug-1026: a resource-UNION element is carried as a copy of its 16-byte
    /// `{tag, record-ptr}` box (the record is still not copied): the element's own
    /// box is a span of its collection's block.
    BorrowedResource,
}

/// How much to copy for a resource live slot that points into the sender's arena
/// (bug-464): a size the descriptor knows at compile time, or a NUL-terminated
/// string measured at run time.
#[derive(Clone, Copy)]
enum BlockLength {
    Fixed(usize),
    CString,
}

/// bug-565: what a raw result tag says about who owns the `Error` behind it, at
/// the moment an inline `TRAP` assembles its `Result`.
///
/// The trapped-error assembly ADOPTS on one tag and REBUILDS on the others, and
/// then frees what it ended up holding. Getting that partition wrong in the
/// adopt direction is a double free, so it is written once, here, as a total
/// function over the tag vocabulary — `tests::the_result_tag_partition_is_total`
/// pins it against the `RESULT_*_TAG` constants actually declared in
/// `error_constants.rs`, so adding a fifth tag reds a test instead of silently
/// falling into `LooseRegisters`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrappedErrorTagClass {
    /// `RESULT_OK_TAG` — not an error at all; the wrap-error branch is not taken.
    NotAnError,
    /// `RESULT_ERR_BLOCK_TAG` — the raiser built ONE owned flat `Error` block and
    /// PARKED it in the per-thread current-error slot for the catcher to adopt
    /// (design "b"). Rebuilding here orphans it, which is half of bug-565.
    ParkedBlock,
    /// `RESULT_ERR_TAG` (a legacy/OOM-degraded loose error) and
    /// `RESULT_PROGRAM_EXIT_TAG`. No block is parked; the flat `Error` must be
    /// rebuilt from the registers. A program exit never actually reaches the
    /// assembly: every inline-`TRAP` lowering re-propagates it first
    /// (`emit_inline_trap_program_exit_passthrough`, bug-841) — its message
    /// register is zero, so a rebuild would dereference null.
    LooseRegisters,
}

/// The class of `tag`, which must be one of the `RESULT_*_TAG` constants.
pub(crate) fn trapped_error_tag_class(tag: &str) -> TrappedErrorTagClass {
    match tag {
        RESULT_OK_TAG => TrappedErrorTagClass::NotAnError,
        RESULT_ERR_BLOCK_TAG => TrappedErrorTagClass::ParkedBlock,
        RESULT_ERR_TAG | RESULT_PROGRAM_EXIT_TAG => TrappedErrorTagClass::LooseRegisters,
        // Unreachable for the declared vocabulary (the test above proves the
        // vocabulary is exactly those four). An unknown tag is treated as a loose
        // error: it REBUILDS, which leaks at worst, rather than adopting a slot
        // nobody parked and freeing a block someone else owns.
        _ => TrappedErrorTagClass::LooseRegisters,
    }
}

/// The one tag whose error arrives as a parked, adoptable block — the emitter's
/// compare. [`trapped_error_tag_class`] classifies exactly this tag as
/// `ParkedBlock`, and `tests::exactly_one_tag_is_adoptable_and_the_emitter_uses_it`
/// pins that against the declared `RESULT_*_TAG` vocabulary, so the compare the
/// generated code performs cannot drift from the classification without a red test.
fn adoptable_error_tag() -> &'static str {
    RESULT_ERR_BLOCK_TAG
}

/// bug-565: where an inline `TRAP`'s error REBUILD branch gets the `ErrorLoc` it
/// inlines into the flat `Error` block.
///
/// This is the ONLY axis on which the three trapped-`Result` lowerings differ, and
/// the partition is total by construction — [`CodeBuilder::emit_trapped_error_result`]
/// `match`es it, so a fourth lowering cannot reach the shared free without
/// declaring which of these it is. `tests/codegen/codegen_trap_error_free.rs`
/// asserts every variant is reachable and that no other emitter builds a trapped
/// error `Result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrappedErrorSource {
    /// The callee's own origin, arriving in `RESULT_ERROR_SOURCE_REGISTER` and
    /// preserved verbatim: the direct user/`.mfb` callee and the indirect
    /// `FUNC`-value callee (`builder_values/lower_call.rs`, `lower_call_result_value`).
    CalleeRegister,
    /// A fresh `ErrorLoc` for the current inline expression: an inline builtin or
    /// a runtime helper trapped here, which has no MFB-level origin of its own.
    CurrentLocation,
    /// An inline-trapped `thread::waitFor`: the message and origin live in the
    /// thread's block arena (plan-156-A) and are deep-copied into this one before
    /// they are inlined.
    WorkerArena,
}

impl CodeBuilder<'_> {}

/// plan-134-F: where a collection payload keeps its pointer edges
/// ([`CodeBuilder::payload_edge_shape`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PayloadEdgeShape {
    /// The payload word points at a separate block: a nested collection, a `Result`, an
    /// `Error`.
    Pointer,
    /// A record inlined in the data region; its pointer fields are the edges.
    InlineRecord,
    /// A data union inlined in the data region; its active variant's pointer fields are
    /// the edges.
    InlineUnion,
}

#[cfg(test)]
mod tests;
