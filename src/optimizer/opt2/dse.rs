//! Dead-store elimination — a Level-2 Opt2 catalog row
//! (`planning/optimizations.md`): remove a stack-slot store that is fully
//! overwritten before any possible read. Block-local and `sp`-relative only,
//! in the exact safety mold of the store-to-load forwarding peephole: the
//! whole analysis state is a set of *pending* stores, and any instruction the
//! pass does not explicitly model clears it, so mis-modeling can only lose an
//! elimination, never remove a store something still reads.
//!
//! The model: an 8-byte `str [sp, #off]` becomes pending. It dies — provably
//! dead — when another full 8-byte store to the *same* offset arrives with
//! nothing in between that could observe memory: only the pure, memory-free
//! ALU/compare ops may pass through. An `ldr [sp, #off']` whose 8-byte range
//! overlaps a pending store consumes it; a partially-overlapping store, any
//! other memory op, any call, label, branch, or unknown instruction clears
//! the state (a label is a join; sp-slot addresses can be recomputed through
//! other bases, and every such access path starts with an op outside the
//! neutral set). Removing a store never removes its source computation — the
//! DCE row, which runs after, sweeps the stranded feeders.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;

use super::plans::mark::removable_op;

/// Run the DSE row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (2).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    // Pending full-slot sp stores: (byte offset, instruction index).
    let mut pending: Vec<(i64, usize)> = Vec::new();
    let mut dead: Vec<usize> = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        match instruction.op {
            CodeOp::StrU64 if sp_based(instruction) => match numeric_offset(instruction) {
                Some(offset) => {
                    // Same-offset pending store: fully overwritten before any
                    // possible read — dead. A partial (<8-byte-apart) overlap
                    // only stops tracking the older store.
                    pending.retain(|&(pending_offset, pending_index)| {
                        if pending_offset == offset {
                            dead.push(pending_index);
                            false
                        } else {
                            (pending_offset - offset).abs() >= 8
                        }
                    });
                    pending.push((offset, index));
                }
                None => pending.clear(),
            },
            CodeOp::LdrU64 if sp_based(instruction) => match numeric_offset(instruction) {
                Some(offset) => {
                    // A read consumes every pending store its 8 bytes overlap.
                    pending.retain(|&(pending_offset, _)| (pending_offset - offset).abs() >= 8);
                }
                None => pending.clear(),
            },
            // Pure ALU plus the memory-free flag-setters may sit between the
            // two stores without observing memory.
            op if removable_op(op)
                || matches!(
                    op,
                    CodeOp::Adds | CodeOp::Subs | CodeOp::Cmp | CodeOp::CmpImm
                ) => {}
            // Everything else — other loads/stores, calls, branches, labels,
            // FP ops, unknowns — might observe the slot: forget everything.
            _ => pending.clear(),
        }
    }
    let removed = dead.len() as u64;
    if removed != 0 {
        let mut keep = vec![true; instructions.len()];
        for index in dead {
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
    /// load of the slot, a call, a label (join), or an unmodeled op.
    #[test]
    fn observed_stores_stay() {
        for observer in [
            load("%v9", "8"),
            ci("bl", &[("target", "callee")]),
            ci("label", &[("name", "join")]),
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
