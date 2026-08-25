//! Dead-store elimination — a Level-2 Opt2 catalog row
//! (`planning/optimizations.md`): remove a stack-slot store that is fully
//! overwritten before any possible read, on **every** path. The dataflow
//! form: a backward must-analysis over the allocator's own CFG
//! (`analysis::build_cfg` — the same blocks its liveness trusts), in the
//! exact safety mold of the store-to-load forwarding peephole: the analysis
//! state is the set of *dead slots* (slots certain to be fully overwritten
//! before any read from here on), and any instruction the pass does not
//! explicitly model clears it, so mis-modeling can only lose an elimination,
//! never remove a store something still reads.
//!
//! The model: an 8-byte `str [sp, #off]` marks its slot dead going backward;
//! an `ldr [sp, #off']` whose 8-byte range overlaps un-marks every slot it
//! touches; a partially-overlapping store conservatively un-marks (no
//! byte-granular credit); only the pure, memory-free ALU/compare ops, labels,
//! and branch terminators pass through untouched — a branch touches no
//! memory, and its *paths* are what the block meet handles: a block's out-set
//! is the **intersection** of its successors' in-sets (a store is dead only
//! if every path overwrites first), and a block with no successor starts
//! empty. Any other memory op, call, or unknown instruction clears the state
//! (sp-slot addresses can be recomputed through other bases, and every such
//! access path starts with an op outside the neutral set). A store proves
//! dead when its own slot is already in the dead set at its point. Removing
//! a store never removes its source computation — the DCE row, which runs
//! after, sweeps the stranded feeders.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{build_cfg, is_block_terminator};
use crate::codegen::engine::types::CodeInstruction;

use super::plans::mark::removable_op;

/// The dead-slot sets are dense bitsets over the sorted slot universe — one
/// bit per distinct full-slot store offset, `nb` sets total. Generated
/// functions reach tens of thousands of blocks with thousands of slots, so
/// per-block set-of-integers state (the first implementation used `BTreeSet`)
/// spends longer *initializing* than the whole rest of the build; word ops
/// keep the fixpoint near-linear.
struct Slots {
    /// Sorted distinct offsets; bit `i` of a set means `offsets[i]` is dead.
    offsets: Vec<i64>,
    words: usize,
}

impl Slots {
    /// Bit indices of every slot whose 8 bytes overlap an access at `offset`
    /// (`|slot - offset| < 8`), as a half-open index range into `offsets`.
    fn overlap(&self, offset: i64) -> std::ops::Range<usize> {
        let start = self.offsets.partition_point(|&o| o <= offset - 8);
        let end = self.offsets.partition_point(|&o| o < offset + 8);
        start..end
    }

    fn index_of(&self, offset: i64) -> Option<usize> {
        self.offsets.binary_search(&offset).ok()
    }

    fn full(&self) -> Vec<u64> {
        let mut bits = vec![u64::MAX; self.words];
        let tail = self.offsets.len() % 64;
        if tail != 0 {
            if let Some(last) = bits.last_mut() {
                *last = (1u64 << tail) - 1;
            }
        }
        bits
    }
}

fn set_bit(bits: &mut [u64], i: usize) {
    bits[i / 64] |= 1u64 << (i % 64);
}

fn clear_bit(bits: &mut [u64], i: usize) {
    bits[i / 64] &= !(1u64 << (i % 64));
}

fn test_bit(bits: &[u64], i: usize) -> bool {
    bits[i / 64] & (1u64 << (i % 64)) != 0
}

/// Run the DSE row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (2).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        crate::optimizer::stats::count_dead_stores_eliminated(0);
        return;
    }

    // The slot universe: every full-slot sp store offset in the function.
    let mut offsets: Vec<i64> = instructions
        .iter()
        .filter(|instruction| instruction.op == CodeOp::StrU64 && sp_based(instruction))
        .filter_map(numeric_offset)
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    if offsets.is_empty() {
        crate::optimizer::stats::count_dead_stores_eliminated(0);
        return;
    }
    let slots = Slots {
        words: offsets.len().div_ceil(64),
        offsets,
    };

    // Backward must-dataflow to a fixpoint: `dead_in[b]` is the dead-slot set
    // at the top of block `b`. Initialize interior facts optimistically to the
    // universe; the meet (intersection over successors) and the transfer only
    // ever shrink them, so the loop terminates at the greatest fixpoint. The
    // out-set of a block with no successor is empty (anything may inspect
    // memory after the function returns to the runtime).
    let nb = blocks.len();
    let mut dead_in: Vec<Vec<u64>> = vec![slots.full(); nb];
    let out_set = |block: usize, dead_in: &[Vec<u64>]| -> Vec<u64> {
        let mut succ = blocks[block].succ.iter();
        let Some(&first) = succ.next() else {
            return vec![0; slots.words];
        };
        let mut dead = dead_in[first].clone();
        for &s in succ {
            for (word, other) in dead.iter_mut().zip(&dead_in[s]) {
                *word &= other;
            }
        }
        dead
    };
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut dead = out_set(b, &dead_in);
            for i in (blocks[b].start..blocks[b].end).rev() {
                transfer(&instructions[i], &slots, &mut dead);
            }
            if dead != dead_in[b] {
                dead_in[b] = dead;
                changed = true;
            }
        }
    }

    // Marking walk: the same backward transfer, now recording each full-slot
    // store whose slot is already dead at its point.
    let mut dead_stores: Vec<usize> = Vec::new();
    for b in 0..nb {
        let mut dead = out_set(b, &dead_in);
        for i in (blocks[b].start..blocks[b].end).rev() {
            let instruction = &instructions[i];
            if instruction.op == CodeOp::StrU64 && sp_based(instruction) {
                if let Some(index) = numeric_offset(instruction).and_then(|o| slots.index_of(o)) {
                    if test_bit(&dead, index) {
                        dead_stores.push(i);
                    }
                }
            }
            transfer(instruction, &slots, &mut dead);
        }
    }

    let removed = dead_stores.len() as u64;
    if removed != 0 {
        let mut keep = vec![true; instructions.len()];
        for index in dead_stores {
            keep[index] = false;
        }
        let mut index = 0;
        instructions.retain(|_| {
            let keep = keep[index];
            index += 1;
            keep
        });
    }
    crate::optimizer::stats::count_dead_stores_eliminated(removed);
}

/// Apply one instruction's backward transfer to the dead-slot bitset.
fn transfer(instruction: &CodeInstruction, slots: &Slots, dead: &mut [u64]) {
    match instruction.op {
        CodeOp::StrU64 if sp_based(instruction) => match numeric_offset(instruction) {
            Some(offset) => {
                // Going backward past a full store, its slot is dead (the
                // store overwrites all 8 bytes without reading). A partially
                // overlapping slot gets no byte-granular credit: un-mark it.
                for i in slots.overlap(offset) {
                    if slots.offsets[i] != offset {
                        clear_bit(dead, i);
                    }
                }
                if let Some(i) = slots.index_of(offset) {
                    set_bit(dead, i);
                }
            }
            None => dead.fill(0),
        },
        CodeOp::LdrU64 if sp_based(instruction) => match numeric_offset(instruction) {
            Some(offset) => {
                // A read revives every slot its 8 bytes overlap.
                for i in slots.overlap(offset) {
                    clear_bit(dead, i);
                }
            }
            None => dead.fill(0),
        },
        CodeOp::Label => {}
        // Pure ALU plus the memory-free flag-setters pass through, and so do
        // branch terminators (no memory access; their successor paths are the
        // block meet's business).
        op if removable_op(op)
            || is_block_terminator(op)
            || matches!(
                op,
                CodeOp::Adds | CodeOp::Subs | CodeOp::Cmp | CodeOp::CmpImm
            ) => {}
        // Everything else — other loads/stores, calls, FP ops, unknowns —
        // might observe the slots: forget everything.
        _ => dead.fill(0),
    }
}

fn sp_based(instruction: &CodeInstruction) -> bool {
    instruction
        .operand("base")
        .is_some_and(|base| base.rendered() == "sp")
}

fn numeric_offset(instruction: &CodeInstruction) -> Option<i64> {
    instruction.get("offset")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn store(src: &str, offset: &str) -> CodeInstruction {
        ci(
            "str_u64",
            &[("src", src), ("base", "sp"), ("offset", offset)],
        )
    }

    fn load(dst: &str, offset: &str) -> CodeInstruction {
        ci(
            "ldr_u64",
            &[("dst", dst), ("base", "sp"), ("offset", offset)],
        )
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        with_opt_level(OptLevel(level), || eliminate(stream));
    }

    /// A store fully overwritten with only pure ALU in between is dead; the
    /// overwriting store stays.
    #[test]
    fn overwritten_store_dies() {
        let mut stream = vec![
            store("%v1", "8"),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v1"), ("imm", "1")]),
            store("%v2", "8"),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream.len(), 3);
        assert_eq!(stream[1].op, CodeOp::StrU64);
        assert_eq!(stream[1].get("src").as_deref(), Some("%v2"));
    }

    /// Any potential observation between the stores keeps the first one: a
    /// load of the slot, a call, or an unmodeled op.
    #[test]
    fn observed_stores_stay() {
        for observer in [
            load("%v9", "8"),
            ci("bl", &[("target", "callee")]),
            ci("fadd_d", &[("dst", "%f1"), ("lhs", "%f2"), ("rhs", "%f3")]),
        ] {
            let mut stream = vec![
                store("%v1", "8"),
                observer,
                store("%v2", "8"),
                ci("ret", &[]),
            ];
            let before = stream.len();
            run(&mut stream, 2);
            assert_eq!(stream.len(), before, "store must survive an observer");
        }
    }

    /// The dataflow form sees through control flow: a store overwritten on
    /// *every* path (both arms of a diamond) is dead across the branch.
    #[test]
    fn store_overwritten_on_all_paths_dies() {
        let mut stream = vec![
            store("%v1", "8"),
            ci("b.eq", &[("target", "else")]),
            store("%v2", "8"),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "else")]),
            store("%v3", "8"),
            ci("label", &[("name", "join")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream.len(), 7, "only the first store dies");
        assert_ne!(stream[0].op, CodeOp::StrU64);
    }

    /// A read reachable on *one* path keeps the store, even though the other
    /// path overwrites it first — the meet is an intersection.
    #[test]
    fn store_read_on_one_path_stays() {
        let mut stream = vec![
            store("%v1", "8"),
            ci("b.eq", &[("target", "join")]),
            load("%v9", "8"),
            ci("label", &[("name", "join")]),
            store("%v2", "8"),
            ci("ret", &[]),
        ];
        let before = stream.len();
        run(&mut stream, 2);
        assert_eq!(stream.len(), before, "a possible read keeps the store");
    }

    /// A store consumed by a load in a later loop iteration survives: the
    /// back edge carries the read into the store's out-set.
    #[test]
    fn loop_carried_read_keeps_the_store() {
        let mut stream = vec![
            ci("label", &[("name", "head")]),
            load("%v1", "8"),
            store("%v2", "8"),
            ci("b.ne", &[("target", "head")]),
            store("%v3", "8"),
            ci("ret", &[]),
        ];
        let before = stream.len();
        run(&mut stream, 2);
        assert_eq!(
            stream.len(),
            before,
            "the loop store feeds the next iteration's load"
        );
    }

    /// A partially-overlapping store is not a full kill: the older store
    /// survives (its unwritten bytes are still observable).
    #[test]
    fn partial_overlap_is_not_a_kill() {
        let mut stream = vec![store("%v1", "8"), store("%v2", "12"), ci("ret", &[])];
        let before = stream.len();
        run(&mut stream, 2);
        assert_eq!(stream.len(), before);
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![store("%v1", "8"), store("%v2", "8"), ci("ret", &[])];
        let before = stream.len();
        run(&mut stream, 1);
        assert_eq!(stream.len(), before);
    }
}
