//! The machine finishing hooks: the choke points every function passes through
//! once its machine code is final.
//!
//! [`finish`] (plan-171-A §4.4) runs at the end of `finalize_frame`
//! (`crate::codegen::engine::util::finalize_frame`), on both of its return paths
//! — a framed function after its prologue and epilogues are in place, and a
//! frameless leaf — so user functions and the vreg-allocated runtime helpers
//! alike reach it, with every `sp` offset final. The rows that need physical
//! registers *and* final offsets run from here, each gated on its own level
//! and on what the backend can encode.
//!
//! [`finish_selected`] (plan-171-B) runs at the end of
//! `mir::route_function_through_mir`, after the last `Backend::select` of every
//! function — hand-written bodies included. The stream is final there: nothing
//! re-selects, re-allocates or re-frames it, and every register is the
//! target's own. The rows that rewrite terminators run from here, so the
//! register allocator never meets what they form.

use super::{cbranch, pairing};
use crate::codegen::engine::types::CodeInstruction;

/// Run the post-frame machine rows over one finalized function, in place.
pub(crate) fn finish(instructions: &mut Vec<CodeInstruction>) {
    if crate::codegen::engine::mir::active_backend().supports_pair_access() {
        let formed = pairing::pair_adjacent_accesses(instructions);
        crate::optimizer::stats::count_load_store_pairs(formed);
    }
}

/// Run the final-stream machine rows over one selected function, in place.
/// `symbol` is the function's symbol: a compiled MFBASIC function
/// (`_mfb_fn_*`, `_mfb_ifn_*`) follows the internal convention at its `ret`.
pub(crate) fn finish_selected(instructions: &mut Vec<CodeInstruction>, symbol: &str) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let backend = crate::codegen::engine::mir::active_backend();
    let formed = cbranch::form(
        instructions,
        backend.compare_branch_forms(),
        backend.register_model(),
        symbol.starts_with("_mfb_fn_") || symbol.starts_with("_mfb_ifn_"),
    );
    crate::optimizer::stats::count_compare_branches(
        formed.zero_branches + formed.bit_tests + formed.zero_compares,
    );
}
