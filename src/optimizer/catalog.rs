//! The landed-row catalog: the single code-level description of every
//! optimization pass the compiler ships, in pipeline order.
//!
//! Two consumers render from it, so the pass list can never fork: the build
//! CLI's `-v` fire-count lines (`Reporter::opt_stats`) and the
//! `mfb man optimizations` guide page, whose pass table is substituted from
//! [`render_markdown_table`] at display time (the `{{optimizer-catalog}}`
//! marker in `src/docs/man/optimizations/package.md`). The forward-looking
//! design catalog — every row considered, landed or not — stays in
//! `planning/optimizations.md`; this module describes only what ships.

use std::sync::atomic::{AtomicU64, Ordering};

use super::stats;

/// One landed dial row.
pub(crate) struct Row {
    /// The catalog row name (matches `planning/optimizations.md`).
    pub(crate) name: &'static str,
    /// The dial level that enables the row (`level_enabled`).
    pub(crate) level: u8,
    /// Where the pass runs: `NIR` (structured native IR, before storage
    /// planning), `MIR` (selected stream, before register allocation),
    /// `machine` (after register allocation).
    pub(crate) stage: &'static str,
    /// One-line user-facing description for the man page table.
    pub(crate) summary: &'static str,
    /// The row's process-wide fire counter (`optimizer::stats`).
    counter: &'static AtomicU64,
}

impl Row {
    /// How many times the row has fired so far in this process.
    pub(crate) fn fired(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}

/// Every landed dial row, in pipeline order.
pub(crate) fn rows() -> &'static [Row] {
    static ROWS: &[Row] = &[
        Row {
            name: "Constant folding",
            level: 1,
            stage: "NIR + MIR",
            summary: "Evaluates constant expressions at compile time — only when \
                      provably non-trapping. A constant that would trap at runtime \
                      (`MAX + 1`, `1 / 0`) is left in place so the trap still fires \
                      exactly as written.",
            counter: &stats::CONSTANT_FOLDING,
        },
        Row {
            name: "Algebraic simplification",
            level: 1,
            stage: "NIR",
            summary: "Identity rewrites: `x * 1`, `x + 0`, `x / 1`, `x ^ 1`, \
                      `s & \"\"`, `b AND TRUE`, `NOT NOT b`, and the exact-IEEE \
                      Float subset (`x * 1.0`, `x / 1.0`, `x - 0.0`).",
            counter: &stats::ALGEBRAIC_SIMPLIFICATION,
        },
        Row {
            name: "Strength reduction (non-loop)",
            level: 1,
            stage: "NIR",
            summary: "Replaces an expensive checked operation with a cheaper one \
                      raising the identical error: `x * 2` becomes `x + x`, \
                      `x ^ 2` becomes `x * x`.",
            counter: &stats::STRENGTH_REDUCTION,
        },
        Row {
            name: "Constant propagation",
            level: 2,
            stage: "MIR",
            summary: "Replaces registers proven (on SSA) to hold the same \
                      constant on every path — across branches and joins the \
                      block-local folder cannot see past.",
            counter: &stats::CONSTANT_PROPAGATION,
        },
        Row {
            name: "Copy propagation",
            level: 2,
            stage: "MIR",
            summary: "Rewrites uses of a copied register to read the copy's \
                      original source directly, when it provably still holds \
                      the same value; the bypassed copies then die as dead code.",
            counter: &stats::COPY_PROPAGATION,
        },
        Row {
            name: "Local value numbering",
            level: 3,
            stage: "MIR",
            summary: "Rewrites a computation repeated within a block — same \
                      operation over operands holding the same values — into a \
                      copy of the earlier result.",
            counter: &stats::LOCAL_VALUE_NUMBERING,
        },
        Row {
            name: "Global value numbering (GVN)",
            level: 3,
            stage: "MIR",
            summary: "Whole-function form of the same idea, on SSA values: a \
                      computation already performed on every path leading here \
                      (a dominating block) becomes a copy of that result.",
            counter: &stats::GLOBAL_VALUE_NUMBERING,
        },
        Row {
            name: "Branch simplification / folding",
            level: 2,
            stage: "NIR + MIR",
            summary: "Folds a branch whose outcome is known at compile time — \
                      `IF TRUE`/`IF FALSE` keeps only its taken arm, \
                      `WHILE FALSE` vanishes, and a compare of known constants \
                      turns its conditional branch into straight-line flow.",
            counter: &stats::BRANCH_SIMPLIFICATION,
        },
        Row {
            name: "Jump threading",
            level: 3,
            stage: "MIR",
            summary: "Redirects a branch that lands on an unconditional jump \
                      straight to the final destination, collapsing \
                      jump-to-jump chains.",
            counter: &stats::JUMP_THREADING,
        },
        Row {
            name: "Dead-code elimination (DCE)",
            level: 2,
            stage: "NIR + MIR",
            summary: "Removes *dead* code: reachable but unused, and provably pure \
                      and trap-free — unused scalar bindings, stranded pure \
                      instructions. An unused `x + y` stays: it can still raise \
                      `ErrOverflow`.",
            counter: &stats::DEAD_CODE_ELIMINATION,
        },
        Row {
            name: "Aggressive DCE (ADCE)",
            level: 3,
            stage: "MIR",
            summary: "Control-dependence-based elimination: removes the dead \
                      *control structure* (a conditional branch guarding only dead \
                      code) that plain DCE leaves behind. Trap-capable regions \
                      always keep their guarding branches.",
            counter: &stats::AGGRESSIVE_DCE,
        },
        Row {
            name: "Unreachable code elimination",
            level: 2,
            stage: "NIR + MIR",
            summary: "Removes *unreachable* code — statements after a RETURN/FAIL \
                      or an always-terminating IF/MATCH, and CFG blocks nothing \
                      jumps to. No trap gate applies: code that never executes \
                      can never raise.",
            counter: &stats::UNREACHABLE_ELIMINATION,
        },
        Row {
            name: "Dead-store elimination",
            level: 2,
            stage: "MIR",
            summary: "Removes a stack-slot store fully overwritten before any \
                      possible read, with only provably memory-free instructions \
                      in between.",
            counter: &stats::DEAD_STORE_ELIMINATION,
        },
        Row {
            name: "Basic block merging",
            level: 2,
            stage: "MIR",
            summary: "Fuses single-predecessor/single-successor block pairs \
                      back into straight-line code: a branch to the very next \
                      block and a label nothing references both vanish.",
            counter: &stats::BLOCK_MERGING,
        },
        Row {
            name: "Peephole optimization (store-to-load forwarding)",
            level: 1,
            stage: "machine",
            summary: "Rewrites a stack reload into a register move when the storing \
                      register provably still holds the value.",
            counter: &stats::PEEPHOLE_FORWARDS,
        },
        Row {
            name: "Machine copy propagation / redundant-move elimination",
            level: 1,
            stage: "machine",
            summary: "Folds a float value's general-register shuttle pair (`fmov` + \
                      store, or load + `fmov`) into one FP memory operation when \
                      the shuttle register is dead.",
            counter: &stats::FP_SHUTTLES_FOLDED,
        },
    ];
    ROWS
}

/// The `mfb man optimizations` pass table, rendered from the catalog.
pub(crate) fn render_markdown_table() -> String {
    let mut table =
        String::from("| Pass | Level | Stage | What it does |\n| --- | --- | --- | --- |\n");
    for row in rows() {
        // The summaries are indented continuation strings; collapse the
        // multi-space runs that literal concatenation leaves behind.
        let summary = row.summary.split_whitespace().collect::<Vec<_>>().join(" ");
        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            row.name, row.level, row.stage, summary
        ));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rendered table carries one line per catalog row plus the two header
    /// lines, and the substitution marker's spelling stays in sync with the
    /// man page source.
    #[test]
    fn table_renders_every_row() {
        let table = render_markdown_table();
        assert_eq!(table.lines().count(), rows().len() + 2);
        for row in rows() {
            assert!(table.contains(row.name), "missing {}", row.name);
        }
        let page = include_str!("../docs/man/optimizations/package.md");
        assert!(
            page.contains("{{optimizer-catalog}}"),
            "the man page must carry the substitution marker"
        );
    }
}
