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
            name: "Loop-invariant code motion (LICM)",
            level: 3,
            stage: "NIR",
            summary: "Moves a binding whose (provably pure, non-trapping) \
                      initializer computes the same value every iteration out \
                      in front of the loop, so it runs once.",
            counter: &stats::LICM_HOISTS,
        },
        Row {
            name: "Loop unswitching",
            level: 3,
            stage: "NIR",
            summary: "Splits a loop testing a loop-invariant condition every \
                      iteration into one up-front test selecting between two \
                      specialized loop copies.",
            counter: &stats::LOOP_UNSWITCHES,
        },
        Row {
            name: "Loop fusion (jamming)",
            level: 3,
            stage: "NIR",
            summary: "Merges adjacent FOR loops over the identical range into \
                      one loop, when their bodies are provably independent, \
                      pure, and trap-free.",
            counter: &stats::LOOPS_FUSED,
        },
        Row {
            name: "Loop fission (distribution)",
            level: 3,
            stage: "NIR",
            summary: "Splits one FOR loop into two over the same range when \
                      its body separates into independent, pure, trap-free \
                      phases.",
            counter: &stats::LOOPS_SPLIT,
        },
        Row {
            name: "Loop peeling",
            level: 3,
            stage: "NIR",
            summary: "Splits a small WHILE loop's first iteration out in \
                      front so later passes can specialize it; evaluation \
                      order and counts are preserved exactly.",
            counter: &stats::LOOPS_PEELED,
        },
        Row {
            name: "Loop rotation",
            level: 3,
            stage: "NIR",
            summary: "Converts a head-tested WHILE into the guarded \
                      bottom-tested form (`IF c THEN DO .. UNTIL NOT c`), \
                      saving a branch per iteration; evaluation order and \
                      counts are preserved exactly.",
            counter: &stats::LOOPS_ROTATED,
        },
        Row {
            name: "Sparse conditional constant propagation (SCCP)",
            level: 3,
            stage: "MIR",
            summary: "Propagates constants and reachability together: a branch \
                      decided by constants makes its untaken path dead, and a \
                      value merging only on live paths stays constant.",
            counter: &stats::SCCP_REWRITES,
        },
        Row {
            name: "Induction variable simplification",
            level: 3,
            stage: "MIR",
            summary: "Merges duplicate loop counters — two variables starting \
                      equal and stepping by the same amount in lockstep become \
                      one.",
            counter: &stats::INDUCTION_VARS_MERGED,
        },
        Row {
            name: "Store-to-load forwarding",
            level: 3,
            stage: "MIR",
            summary: "Reads a value from the register that stored it instead of \
                      from the stack, when every path to the load leaves the \
                      slot untouched.",
            counter: &stats::STORES_FORWARDED,
        },
        Row {
            name: "Redundant load elimination",
            level: 3,
            stage: "MIR",
            summary: "Drops a reload of a stack slot an earlier load already \
                      brought into a register, when nothing in between can have \
                      written it.",
            counter: &stats::REDUNDANT_LOADS_REMOVED,
        },
        Row {
            name: "Tail duplication",
            level: 3,
            stage: "MIR",
            summary: "Copies a small shared tail into each block that jumps to \
                      it, so the passes that must forget their facts at a merge \
                      see straight-line code instead.",
            counter: &stats::TAILS_DUPLICATED,
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
            name: "Object / aggregate copy propagation",
            level: 3,
            stage: "NIR",
            summary: "Removes a whole-block copy of a record, String or collection \
                      when neither the copy nor its source is ever written, \
                      address-taken, captured, or a resource owner — so one block \
                      does the work of two and one cleanup frees it.",
            counter: &stats::AGGREGATE_COPIES_FORWARDED,
        },
        Row {
            name: "Recovery-region simplification",
            level: 3,
            stage: "NIR",
            summary: "Removes a `TRAP` handler whose guarded region provably cannot \
                      enter the error path — no call, no checked arithmetic, no \
                      `FAIL`, no closing resource — so nothing can ever branch to it.",
            counter: &stats::RECOVERY_REGIONS_SIMPLIFIED,
        },
        Row {
            name: "String concat / rope fusion",
            level: 3,
            stage: "codegen",
            summary: "Lowers a whole `a & b & c` chain into one pre-sized allocation \
                      and one pass of writes. `&` is left-associative, so the \
                      pairwise form allocates and fills an intermediate String per \
                      operator and copies it again into the next one.",
            counter: &stats::STRING_CONCATS_FUSED,
        },
        Row {
            name: "Codepoint `len()` caching",
            level: 3,
            stage: "NIR",
            summary: "Scans a String's codepoints once and reuses the count. A \
                      String's byte length is an O(1) header read, but `len()` is \
                      the codepoint count and lowers to a loop over every byte, so \
                      two `len(s)` in one run scan the whole string twice.",
            counter: &stats::LEN_CACHES,
        },
        Row {
            name: "Store PRE / Load PRE",
            level: 3,
            stage: "MIR",
            summary: "Completes a stack-slot read that is already in a register on \
                      every path into a join but one, then rewrites the join's own \
                      read as a copy - one load placed for one removed.",
            counter: &stats::MEMORY_PRE,
        },
        Row {
            name: "Partial dead-store elimination",
            level: 3,
            stage: "MIR",
            summary: "Sinks a store that is dead on one branch and live on the other \
                      into the branch that needs it, so the dead path stops writing. \
                      Only where the receiving branch is reachable no other way.",
            counter: &stats::PARTIAL_DEAD_STORES,
        },
        Row {
            name: "Loop-nest invariant code motion",
            level: 3,
            stage: "MIR",
            summary: "Hoists an invariant computation to the shallowest enclosing \
                      loop it is still invariant at, over the loops the machine \
                      actually has — the desugared and inlined ones the structured \
                      NIR pass cannot see. Never a trapping operation.",
            counter: &stats::LOOP_NEST_HOISTS,
        },
        Row {
            name: "Partial redundancy elimination (PRE)",
            level: 3,
            stage: "MIR",
            summary: "Completes an expression already computed on some paths into a \
                      join by computing it on the one path that lacked it, then \
                      deletes the join's own copy. Fires only when the insertion \
                      and the deletion cancel, so the program never grows.",
            counter: &stats::PARTIAL_REDUNDANCIES,
        },
        Row {
            name: "Code sinking",
            level: 3,
            stage: "MIR",
            summary: "Moves a computation down into the single branch that uses it, \
                      so the other path stops paying for it. Only where that branch \
                      is reachable no other way, which makes the move free of any \
                      trip-count guess.",
            counter: &stats::CODE_SINKS,
        },
        Row {
            name: "Load/store hoisting and sinking",
            level: 3,
            stage: "MIR",
            summary: "Moves a stack-slot access into the one branch that uses it, and \
                      the mirror: an identical access leading both arms of a branch \
                      moves up above it, so it is stored once instead of twice.",
            counter: &stats::MEMORY_MOTIONS,
        },
        Row {
            name: "Check fusion with existing comparisons",
            level: 3,
            stage: "MIR",
            summary: "Deletes a comparison the condition flags already reflect, so a \
                      guarded region's second test reuses the guard's own compare \
                      instead of recomputing it. The branch is untouched.",
            counter: &stats::CHECK_FUSIONS,
        },
        Row {
            name: "Correlated value propagation",
            level: 3,
            stage: "MIR",
            summary: "Refines a value from the branch conditions that dominate it, \
                      then uses the refinement: a comparison the dominating \
                      conditions already settle becomes unconditional flow, and a \
                      value they pin to one number becomes that number.",
            counter: &stats::CORRELATED_VALUE_PROPAGATION,
        },
        Row {
            name: "Overflow-check elimination",
            level: 3,
            stage: "MIR",
            summary: "Drops a checked add or subtract's overflow guard when the \
                      operands' proven ranges cannot sum past the 64-bit range. A \
                      guard without such a proof is left exactly where it is, so \
                      every trap that can fire still fires as written.",
            counter: &stats::OVERFLOW_CHECKS_ELIDED,
        },
        Row {
            name: "Division / modulo-check elimination",
            level: 3,
            stage: "MIR",
            summary: "Drops the divisor-is-zero and `MIN / -1` guards when the \
                      divisor's proven range excludes those values.",
            counter: &stats::DIVISION_CHECKS_ELIDED,
        },
        Row {
            name: "Bounds-check elimination",
            level: 3,
            stage: "MIR",
            summary: "Removes an index test whose failing edge raises \
                      `ErrIndexOutOfRange` when a dominating condition already \
                      proves the index in range.",
            counter: &stats::BOUNDS_CHECKS_ELIDED,
        },
        Row {
            name: "Range-check widening / narrowing",
            level: 3,
            stage: "MIR",
            summary: "Carries one proven range through arithmetic to discharge the \
                      checks on values derived from it — a single `i < n` also \
                      settling `i + 1` and `i * 2` — without moving any trap.",
            counter: &stats::RANGE_CHECKS_DERIVED,
        },
        Row {
            name: "Redundant union-tag / error-tag check elimination",
            level: 3,
            stage: "MIR",
            summary: "Removes a discriminant or fallible-result test that an \
                      equivalent dominating test already settled.",
            counter: &stats::TAG_CHECKS_ELIDED,
        },
        Row {
            name: "Dead error-handler / fallible-branch elimination",
            level: 3,
            stage: "MIR",
            summary: "When a guard is proven never to fail, its raise path and \
                      handler are unreachable and the unreachable-code row sweeps \
                      them.",
            counter: &stats::DEAD_ERROR_HANDLERS_REMOVED,
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
            name: "Alignment optimization",
            level: 2,
            stage: "Plan1",
            summary: "Orders constant and writable data so each object lands \
                      already-aligned behind the previous one, removing the \
                      padding a narrow-then-wide order wastes.",
            counter: &stats::ALIGNMENT_BYTES_SAVED,
        },
        Row {
            name: "CFG simplification (simplifycfg)",
            level: 2,
            stage: "MIR",
            summary: "Structural control-flow tidying: a conditional branch \
                      whose two edges land in the same place, a jump to a \
                      block that only returns, and duplicate labels naming one \
                      point.",
            counter: &stats::CFG_SIMPLIFICATIONS,
        },
        Row {
            name: "Known-bits simplification",
            level: 2,
            stage: "MIR",
            summary: "Uses what is provably known about each bit of a value to \
                      replace an operation with its constant result, or with a \
                      copy when it cannot change its input.",
            counter: &stats::KNOWN_BITS_SIMPLIFICATIONS,
        },
        Row {
            name: "Narrowing / bit-width reduction",
            level: 2,
            stage: "MIR",
            summary: "Drops a mask whose bits the value provably already \
                      satisfies — the value was already narrow.",
            counter: &stats::VALUES_NARROWED,
        },
        Row {
            name: "Sign/zero extension elimination",
            level: 2,
            stage: "MIR",
            summary: "Drops a widening whose high bits are provably already \
                      clear, so the extension cannot change the value.",
            counter: &stats::EXTENSIONS_REMOVED,
        },
        Row {
            name: "Dead global elimination",
            level: 2,
            stage: "NIR",
            summary: "Removes a private global nothing in the program ever \
                      reads or writes.",
            counter: &stats::GLOBALS_ELIMINATED,
        },
        Row {
            name: "Global localization / constification",
            level: 2,
            stage: "NIR",
            summary: "Replaces reads of a private global that is never written \
                      with its constant initializer, turning a memory load into \
                      an immediate the folding rows can see through.",
            counter: &stats::GLOBALS_LOCALIZED,
        },
        Row {
            name: "Read-only memory inference",
            level: 2,
            stage: "NIR",
            summary: "Proves a private global is never written after \
                      initialization and marks it immutable, so storage \
                      planning may place it in read-only memory.",
            counter: &stats::GLOBALS_READ_ONLY,
        },
        Row {
            name: "Spill-code optimization",
            level: 2,
            stage: "regalloc",
            summary: "Deletes a reload whose value is already sitting in the \
                      target register — the redundancy that arises from \
                      emitting a reload before every use independently.",
            counter: &stats::SPILL_CODE_REMOVED,
        },
        Row {
            name: "Register coalescing",
            level: 2,
            stage: "regalloc",
            summary: "Gives a copy's source and destination the same register \
                      when they never hold different values, so the copy \
                      disappears entirely.",
            counter: &stats::REGISTERS_COALESCED,
        },
        Row {
            name: "Rematerialization",
            level: 2,
            stage: "regalloc",
            summary: "Recomputes a spilled constant at each use instead of \
                      storing it to the stack and loading it back.",
            counter: &stats::VALUES_REMATERIALIZED,
        },
        Row {
            name: "Stack slot coloring",
            level: 2,
            stage: "regalloc",
            summary: "Shares one stack slot between spilled values whose \
                      lifetimes do not overlap, shrinking the stack frame.",
            counter: &stats::SPILL_SLOTS_SHARED,
        },
        Row {
            name: "Live-range splitting",
            level: 2,
            stage: "regalloc",
            summary: "Keeps a value in registers for its whole life by giving \
                      it one register for the first part and another for the \
                      rest, instead of spilling it to memory throughout.",
            counter: &stats::LIVE_RANGES_SPLIT,
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
