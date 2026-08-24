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
static CONSTANT_FOLDING: AtomicU64 = AtomicU64::new(0);
/// "Algebraic simplification" (Opt1): identity rewrites applied.
static ALGEBRAIC_SIMPLIFICATION: AtomicU64 = AtomicU64::new(0);
/// "Strength reduction (non-loop)" (Opt1): checked ops replaced by cheaper
/// trap-identical ones.
static STRENGTH_REDUCTION: AtomicU64 = AtomicU64::new(0);
/// "Peephole optimization" (post-regalloc): stack reloads forwarded to a
/// register move by `forward_stores_to_loads`.
static PEEPHOLE_FORWARDS: AtomicU64 = AtomicU64::new(0);
/// "Machine copy propagation / redundant-move elimination" (post-regalloc):
/// FP shuttle pairs folded by `remove_fp_shuttles`.
static FP_SHUTTLES_FOLDED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn count_constant_folds(fired: u64) {
    add(&CONSTANT_FOLDING, fired);
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

/// `(catalog row label, fires so far)` for every landed dial row, in catalog
/// order — the lines `mfb build -v` prints after codegen.
pub(crate) fn snapshot() -> [(&'static str, u64); 5] {
    [
        ("Constant folding", CONSTANT_FOLDING.load(Ordering::Relaxed)),
        (
            "Algebraic simplification",
            ALGEBRAIC_SIMPLIFICATION.load(Ordering::Relaxed),
        ),
        (
            "Strength reduction (non-loop)",
            STRENGTH_REDUCTION.load(Ordering::Relaxed),
        ),
        (
            "Peephole optimization (store-to-load forwarding)",
            PEEPHOLE_FORWARDS.load(Ordering::Relaxed),
        ),
        (
            "Machine copy propagation / redundant-move elimination",
            FP_SHUTTLES_FOLDED.load(Ordering::Relaxed),
        ),
    ]
}
