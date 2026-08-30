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
/// "Loop-invariant code motion (LICM)" (Opt1): invariant pure binds moved in
/// front of their loops.
pub(super) static LICM_HOISTS: AtomicU64 = AtomicU64::new(0);
/// "Loop unswitching" (Opt1): loops split on an invariant condition.
pub(super) static LOOP_UNSWITCHES: AtomicU64 = AtomicU64::new(0);
/// "Loop fusion (jamming)" (Opt1): adjacent identical-range loops merged.
pub(super) static LOOPS_FUSED: AtomicU64 = AtomicU64::new(0);
/// "Loop fission (distribution)" (Opt1): loops split into independent phases.
pub(super) static LOOPS_SPLIT: AtomicU64 = AtomicU64::new(0);
/// "Loop peeling" (Opt1): first iterations split out in front.
pub(super) static LOOPS_PEELED: AtomicU64 = AtomicU64::new(0);
/// "Loop rotation" (Opt1): head-tested loops converted to the guarded
/// bottom-tested form.
pub(super) static LOOPS_ROTATED: AtomicU64 = AtomicU64::new(0);
/// "Sparse conditional constant propagation (SCCP)" (Opt2): instructions
/// rewritten to constants and branches decided by the optimistic
/// constant+reachability fixpoint.
pub(super) static SCCP_REWRITES: AtomicU64 = AtomicU64::new(0);
/// "Induction variable simplification" (Opt2): uses redirected from a
/// duplicate loop counter to its surviving twin.
pub(super) static INDUCTION_VARS_MERGED: AtomicU64 = AtomicU64::new(0);
/// "Store-to-load forwarding" (Opt2, L3): loads rewritten to a copy of the
/// storing register across the CFG.
pub(super) static STORES_FORWARDED: AtomicU64 = AtomicU64::new(0);
/// "Redundant load elimination" (Opt2): reloads rewritten to a copy of an
/// earlier load's register.
pub(super) static REDUNDANT_LOADS_REMOVED: AtomicU64 = AtomicU64::new(0);
/// "Tail duplication" (Opt2): small join tails copied into their
/// predecessors, removing the merge for the downstream block-local rows.
pub(super) static TAILS_DUPLICATED: AtomicU64 = AtomicU64::new(0);
/// "Local value numbering" (Opt2): block-local recomputes rewritten to
/// copies of the earlier result.
pub(super) static LOCAL_VALUE_NUMBERING: AtomicU64 = AtomicU64::new(0);
/// "Global value numbering (GVN)" (Opt2): dominated recomputes rewritten to
/// copies of the dominating result (also the CSE row).
pub(super) static GLOBAL_VALUE_NUMBERING: AtomicU64 = AtomicU64::new(0);
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

pub(crate) fn count_licm_hoists(fired: u64) {
    add(&LICM_HOISTS, fired);
}

pub(crate) fn count_loop_unswitches(fired: u64) {
    add(&LOOP_UNSWITCHES, fired);
}

pub(crate) fn count_loops_fused(fired: u64) {
    add(&LOOPS_FUSED, fired);
}

pub(crate) fn count_loops_split(fired: u64) {
    add(&LOOPS_SPLIT, fired);
}

pub(crate) fn count_loops_peeled(fired: u64) {
    add(&LOOPS_PEELED, fired);
}

pub(crate) fn count_loops_rotated(fired: u64) {
    add(&LOOPS_ROTATED, fired);
}

pub(crate) fn count_sccp_rewrites(fired: u64) {
    add(&SCCP_REWRITES, fired);
}

pub(crate) fn count_induction_vars_merged(fired: u64) {
    add(&INDUCTION_VARS_MERGED, fired);
}

pub(crate) fn count_stores_forwarded(fired: u64) {
    add(&STORES_FORWARDED, fired);
}

pub(crate) fn count_redundant_loads_removed(fired: u64) {
    add(&REDUNDANT_LOADS_REMOVED, fired);
}

pub(crate) fn count_tails_duplicated(fired: u64) {
    add(&TAILS_DUPLICATED, fired);
}

pub(crate) fn count_local_value_numberings(fired: u64) {
    add(&LOCAL_VALUE_NUMBERING, fired);
}

pub(crate) fn count_global_value_numberings(fired: u64) {
    add(&GLOBAL_VALUE_NUMBERING, fired);
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
