//! The machine finishing hook (plan-171-A §4.4): the one choke point every
//! register-allocated function passes through once its frame is final.
//!
//! [`finish`] runs at the end of `finalize_frame`
//! (`crate::codegen::engine::util::finalize_frame`), on both of its return paths
//! — a framed function after its prologue and epilogues are in place, and a
//! frameless leaf — so user functions and the vreg-allocated runtime helpers
//! alike reach it, with every `sp` offset final. The rows that need physical
//! registers *and* final offsets run from here, each gated on its own level
//! and on what the backend can encode.

use super::pairing;
use crate::codegen::engine::types::CodeInstruction;

/// Run the post-frame machine rows over one finalized function, in place.
pub(crate) fn finish(instructions: &mut Vec<CodeInstruction>) {
    if crate::codegen::engine::mir::active_backend().supports_pair_access() {
        let formed = pairing::pair_adjacent_accesses(instructions);
        crate::optimizer::stats::count_load_store_pairs(formed);
    }
}
