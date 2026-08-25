//! Fire counts for the landed dial rows, one process-wide counter per row.
//!
//! Each gated pass adds how many times it actually rewrote something; the build
//! CLI prints one `<catalog row>: <count>` line per row on stderr under
//! `-v`/`--verbose` (`Reporter::opt_stats`). Counters are plain atomics rather
//! than build-scoped state because a pass runs deep inside per-function codegen
//! with no channel back to the CLI; a `mfb build` process runs one build, so
//! process-cumulative equals per-build where the lines are printed.

use std::sync::atomic::{AtomicU64, Ordering};

/// "Constant folding" (Opt1): constant expressions evaluated at compile time.
pub(super) static CONSTANT_FOLDING: AtomicU64 = AtomicU64::new(0);
/// "Algebraic simplification" (Opt1): identity rewrites applied.
pub(super) static ALGEBRAIC_SIMPLIFICATION: AtomicU64 = AtomicU64::new(0);
/// "Strength reduction (non-loop)" (Opt1): checked ops replaced by cheaper
/// trap-identical ones.
pub(super) static STRENGTH_REDUCTION: AtomicU64 = AtomicU64::new(0);
/// "Constant propagation" (Opt2): instructions rewritten to `mov_imm` from
/// SSA-proven cross-block constants.
pub(super) static CONSTANT_PROPAGATION: AtomicU64 = AtomicU64::new(0);
/// "Copy propagation" (Opt2): register uses rewritten to read a copy's
/// ultimate source directly.
pub(super) static COPY_PROPAGATION: AtomicU64 = AtomicU64::new(0);
/// "Branch simplification / folding" (both seams): constant-condition IFs /
/// WHILE FALSE folded on NIR plus known compare-and-branches folded on MIR.
pub(super) static BRANCH_SIMPLIFICATION: AtomicU64 = AtomicU64::new(0);
/// "Jump threading" (Opt2): branch targets redirected past trampoline blocks.
pub(super) static JUMP_THREADING: AtomicU64 = AtomicU64::new(0);
/// "Basic block merging" (Opt2): branches-to-next and orphaned labels fused
/// away.
pub(super) static BLOCK_MERGING: AtomicU64 = AtomicU64::new(0);
/// "Dead-code elimination (DCE)" (both seams): dead binds/evals removed on NIR
/// plus dead pure instructions removed on MIR.
pub(super) static DEAD_CODE_ELIMINATION: AtomicU64 = AtomicU64::new(0);
/// "Aggressive DCE (ADCE)" (Opt2): dead instructions + dead conditional
/// branches removed with control-dependence marking.
pub(super) static AGGRESSIVE_DCE: AtomicU64 = AtomicU64::new(0);
/// "Unreachable code elimination" (both seams): post-terminal statements
/// dropped on NIR plus unreachable CFG blocks pruned on MIR.
pub(super) static UNREACHABLE_ELIMINATION: AtomicU64 = AtomicU64::new(0);
/// "Dead-store elimination" (Opt2): sp-slot stores fully overwritten before
/// any possible read.
pub(super) static DEAD_STORE_ELIMINATION: AtomicU64 = AtomicU64::new(0);
/// "Peephole optimization" (post-regalloc): stack reloads forwarded to a
/// register move by `forward_stores_to_loads`.
pub(super) static PEEPHOLE_FORWARDS: AtomicU64 = AtomicU64::new(0);
/// "Machine copy propagation / redundant-move elimination" (post-regalloc):
/// FP shuttle pairs folded by `remove_fp_shuttles`.
pub(super) static FP_SHUTTLES_FOLDED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn count_constant_folds(fired: u64) {
    add(&CONSTANT_FOLDING, fired);
}

pub(crate) fn count_constant_propagations(fired: u64) {
    add(&CONSTANT_PROPAGATION, fired);
}

pub(crate) fn count_copy_propagations(fired: u64) {
    add(&COPY_PROPAGATION, fired);
}

pub(crate) fn count_branch_simplifications(fired: u64) {
    add(&BRANCH_SIMPLIFICATION, fired);
}

pub(crate) fn count_jumps_threaded(fired: u64) {
    add(&JUMP_THREADING, fired);
}

pub(crate) fn count_blocks_merged(fired: u64) {
    add(&BLOCK_MERGING, fired);
}

pub(crate) fn count_dead_code_eliminations(fired: u64) {
    add(&DEAD_CODE_ELIMINATION, fired);
}

pub(crate) fn count_aggressive_dce(fired: u64) {
    add(&AGGRESSIVE_DCE, fired);
}

pub(crate) fn count_unreachable_eliminations(fired: u64) {
    add(&UNREACHABLE_ELIMINATION, fired);
}

pub(crate) fn count_dead_stores_eliminated(fired: u64) {
    add(&DEAD_STORE_ELIMINATION, fired);
}

pub(crate) fn count_algebraic_simplifications(fired: u64) {
    add(&ALGEBRAIC_SIMPLIFICATION, fired);
}

pub(crate) fn count_strength_reductions(fired: u64) {
    add(&STRENGTH_REDUCTION, fired);
}

pub(crate) fn count_peephole_forwards(fired: u64) {
    add(&PEEPHOLE_FORWARDS, fired);
}

pub(crate) fn count_fp_shuttles_folded(fired: u64) {
    add(&FP_SHUTTLES_FOLDED, fired);
}

fn add(counter: &AtomicU64, fired: u64) {
    if fired != 0 {
        counter.fetch_add(fired, Ordering::Relaxed);
    }
}
