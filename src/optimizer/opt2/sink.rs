//! Code sinking, and load/store hoisting and sinking — two Level-3 Opt2
//! catalog rows (`planning/optimizations.md`) that are one mechanism read in
//! two directions: move work *down* into the branch that actually uses it, and
//! move work *up* out of both branches when they do the same thing.
//!
//! The shape this row fires on is deliberately narrow, and that narrowness is
//! what makes it unconditionally profitable rather than a guess:
//!
//! - the instruction sits in a block `B` that ends in a **conditional**
//!   branch, and
//! - every use of its result lies in blocks dominated by one successor `S`,
//!   and
//! - `S`'s **only** predecessor is `B`.
//!
//! Those three together mean `S` runs at most as often as `B` does — control
//! can reach `S` no other way — so the move can never make the instruction
//! execute more times than before. No trip counts, no profile, no heuristic.
//! (This is also why the row does not need loop analysis: a block whose sole
//! predecessor is `B` cannot be a loop header, because a header always has a
//! second, back edge.)
//!
//! **What may sink.** Pure ALU ops from the shared whitelist, and `sp`-slot
//! loads. A load additionally requires that nothing between it and the end of
//! `B` may write memory, so the value it would load after the move is the
//! same one it loads now. Stores are not sunk — moving a store past a branch
//! changes which paths observe it, which is the *partial* dead-store question,
//! a different row with a different proof obligation.
//!
//! **What may hoist** ([`hoist`]): an identical `sp`-slot load or store
//! leading *both* arms. Both arms already ran it and the branching block runs
//! exactly once per arm entry, so one copy above the branch runs the same
//! number of times — a pure size win with no schedule change. A store may
//! hoist where it may not sink, because hoisting does not change *which* paths
//! observe it.
//!
//! **Why single definition.** The overlay is an overlay: the stream keeps its
//! `%vN` registers and a register may be written more than once. Rather than
//! prove no intervening write on every path, the row requires the moved
//! instruction's destination *and* every operand to have exactly one
//! definition in the whole function. Then no path can redefine them, and the
//! move is a pure re-placement of an unchanged computation. Same
//! holder-currency rule the value-numbering and memory rows use.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{build_cfg, effect, Block, ClassModel, RegRef};
use crate::codegen::engine::regalloc::{self};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::mark::{conditional_terminator, removable_op};
use super::plans::ranges::block_of;
use super::plans::ssa::{self, Ssa};

/// Run the sinking rows over one function's selected stream, in place.
/// Self-guarded on the shared catalog level (3).
pub(crate) fn sink(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.len() < 2 {
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);
    let where_of = block_of(&blocks, instructions.len());

    // How many times each integer vreg is defined anywhere in the stream, and
    // where each is used. One pass; both classes' registers are distinct
    // numbering spaces, and only the integer class is sunk here.
    let (def_count, uses_of) = register_facts(instructions, &models.0);
    let preds = predecessors(&blocks);

    // (source index, destination block) in stream order.
    let mut moves: Vec<(usize, usize)> = Vec::new();
    let mut pure_sinks = 0u64;
    let mut load_sinks = 0u64;

    for block in &blocks {
        let terminator = block.end - 1;
        if !conditional_terminator(instructions[terminator].op) {
            continue;
        }
        // Both edges must go somewhere different, or there is no "the branch
        // that uses it" to sink into.
        if block.succ.len() < 2 || block.succ[0] == block.succ[1] {
            continue;
        }
        for i in block.start..terminator {
            let is_load =
                matches!(instructions[i].op, CodeOp::LdrU64) && sp_based(&instructions[i]);
            if !removable_op(instructions[i].op) && !is_load {
                continue;
            }
            // A load may only move down over instructions that cannot write
            // memory; anything else could change what it reads.
            if is_load && !memory_quiet(instructions, i + 1, terminator) {
                continue;
            }
            let Some(destination) = single_def_vreg(instructions, &models.0, i, &def_count) else {
                continue;
            };
            if !operands_are_stable(instructions, &models.0, i, &def_count) {
                continue;
            }
            let Some(uses) = uses_of.get(&destination) else {
                // No use at all — that is the dead-code row's business, not
                // this one's.
                continue;
            };
            let Some(target) =
                sole_successor_covering(&blocks, &preds, &overlay, block, &where_of, uses)
            else {
                continue;
            };
            moves.push((i, target));
            if is_load {
                load_sinks += 1;
            } else {
                pure_sinks += 1;
            }
        }
    }

    if !moves.is_empty() {
        apply(instructions, &blocks, moves);
    }
    drop((blocks, overlay, where_of, def_count, uses_of, preds));

    // The mirror operation, on whatever stream the sinks left behind: an
    // identical memory access leading BOTH arms of a branch belongs above it,
    // not in each. The facts are rebuilt unconditionally rather than reused,
    // because sinking may have moved instructions out from under them — and
    // when it did not, a rebuild returns the same facts, so there is no case
    // worth a second code path.
    let blocks = build_cfg(instructions);
    let overlay = ssa::build(instructions, &blocks, &models);
    let where_of = block_of(&blocks, instructions.len());
    let (def_count, uses_of) = register_facts(instructions, &models.0);
    let preds = predecessors(&blocks);
    let hoisted = hoist(
        instructions,
        &blocks,
        &overlay,
        &models,
        &where_of,
        &preds,
        &def_count,
        &uses_of,
    );

    crate::optimizer::stats::count_code_sinks(pure_sinks);
    crate::optimizer::stats::count_memory_motions(load_sinks + hoisted);
}

/// Per-block predecessor lists.
fn predecessors(blocks: &[Block]) -> Vec<Vec<usize>> {
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for &successor in &block.succ {
            preds[successor].push(b);
        }
    }
    preds
}

/// How many times each integer vreg is defined, and where each is used.
#[allow(clippy::type_complexity)]
fn register_facts(
    instructions: &[CodeInstruction],
    model: &ClassModel,
) -> (
    std::collections::HashMap<u32, usize>,
    std::collections::HashMap<u32, Vec<usize>>,
) {
    let mut def_count: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut uses_of: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for (i, instruction) in instructions.iter().enumerate() {
        let effect = effect(instruction, model);
        for reference in effect.defs {
            if let RegRef::VReg(id) = reference {
                *def_count.entry(id).or_insert(0) += 1;
            }
        }
        for reference in effect.uses {
            if let RegRef::VReg(id) = reference {
                uses_of.entry(id).or_default().push(i);
            }
        }
    }
    (def_count, uses_of)
}

/// The single successor of `block` that (a) has `block` as its only
/// predecessor and (b) dominates every one of `uses`.
fn sole_successor_covering(
    blocks: &[Block],
    preds: &[Vec<usize>],
    overlay: &Ssa,
    block: &Block,
    where_of: &[usize],
    uses: &[usize],
) -> Option<usize> {
    for &successor in &block.succ {
        if preds[successor].len() != 1 {
            continue;
        }
        if blocks[successor].start >= blocks[successor].end {
            continue;
        }
        if uses
            .iter()
            .all(|&use_index| overlay.dominates(successor, where_of[use_index]))
        {
            return Some(successor);
        }
    }
    None
}

/// The instruction's destination vreg, when it has exactly one and that vreg
/// is defined exactly once in the whole stream.
fn single_def_vreg(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    i: usize,
    def_count: &std::collections::HashMap<u32, usize>,
) -> Option<u32> {
    let effect = effect(&instructions[i], model);
    if effect.defs.len() != 1 || effect.is_call {
        return None;
    }
    match effect.defs[0] {
        RegRef::VReg(id) if def_count.get(&id) == Some(&1) => Some(id),
        _ => None,
    }
}

/// Whether every register the instruction reads is stable across the move —
/// written at most once in the whole stream, so no path between the old and
/// new positions can give it a different value.
///
/// Zero definitions counts as stable too: a vreg nothing in this function
/// writes is a live-in, and a live-in cannot change underneath the move.
/// The stack pointer is likewise stable — it is the one physical register
/// this seam may read, and the frame it addresses is fixed for the function.
fn operands_are_stable(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    i: usize,
    def_count: &std::collections::HashMap<u32, usize>,
) -> bool {
    let instruction = &instructions[i];
    if effect(instruction, model).uses.iter().any(|reference| {
        matches!(reference, RegRef::VReg(id) if def_count.get(id).copied().unwrap_or(0) > 1)
    }) {
        return false;
    }
    // Any *physical* register read other than the stack pointer is an ABI
    // effect with no tracked lifetime, so the move cannot be justified.
    !instruction.fields.iter().any(|(name, operand)| {
        crate::codegen::engine::regalloc::analysis::is_use_field(name)
            && matches!(
                crate::codegen::engine::regalloc::analysis::classify_ref(operand, model),
                Some(RegRef::Phys(_))
            )
            && operand.rendered() != crate::target::shared::abi::stack_pointer()
    })
}

/// Whether `[from, to)` provably writes no memory — the precondition for
/// moving a load down past it.
fn memory_quiet(instructions: &[CodeInstruction], from: usize, to: usize) -> bool {
    instructions[from..to].iter().all(|instruction| {
        removable_op(instruction.op)
            || matches!(
                instruction.op,
                CodeOp::Cmp | CodeOp::CmpImm | CodeOp::Label | CodeOp::LdrU64
            )
    })
}

/// Whether a memory instruction addresses a stack slot — the only memory this
/// seam's rows reason about.
fn sp_based(instruction: &CodeInstruction) -> bool {
    instruction.get("base").as_deref() == Some(crate::target::shared::abi::stack_pointer())
}

/// Rebuild the stream with each moved instruction removed from its old place
/// and re-inserted at the top of its destination block, after that block's
/// leading label. Relative order among instructions sunk into the same block
/// is preserved.
fn apply(instructions: &mut Vec<CodeInstruction>, blocks: &[Block], moves: Vec<(usize, usize)>) {
    // Insertion point per destination block: just past a leading label.
    let insert_at = |block: usize| -> usize {
        let start = blocks[block].start;
        if instructions[start].op == CodeOp::Label {
            start + 1
        } else {
            start
        }
    };
    let mut pending: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut removed = vec![false; instructions.len()];
    for (source, destination) in moves {
        removed[source] = true;
        let point = insert_at(destination);
        match pending.iter_mut().find(|(at, _)| *at == point) {
            Some((_, list)) => list.push(source),
            None => pending.push((point, vec![source])),
        }
    }

    let mut rebuilt: Vec<CodeInstruction> = Vec::with_capacity(instructions.len());
    let taken = std::mem::take(instructions);
    let mut carried: Vec<Option<CodeInstruction>> = taken.into_iter().map(Some).collect();
    for index in 0..carried.len() {
        if let Some((_, sources)) = pending.iter().find(|(at, _)| *at == index) {
            for &source in sources {
                if let Some(instruction) = carried[source].take() {
                    rebuilt.push(instruction);
                }
            }
        }
        if removed[index] {
            continue;
        }
        if let Some(instruction) = carried[index].take() {
            rebuilt.push(instruction);
        }
    }
    // A destination point at the very end of the stream (no instruction index
    // equals it) still has to receive its arrivals.
    if let Some((_, sources)) = pending.iter().find(|(at, _)| *at == carried.len()) {
        for &source in sources {
            if let Some(instruction) = carried[source].take() {
                rebuilt.push(instruction);
            }
        }
    }
    *instructions = rebuilt;
}

/// The hoisting half of load/store hoisting and sinking: an identical memory
/// access at the top of **both** arms of a conditional branch is one access,
/// so it moves up into the branching block and the two copies go.
///
/// It runs the same number of times either way — both arms already ran it, and
/// the block that branches runs exactly once per arm entry — so this is a pure
/// size win with no schedule change. The conditions are the mirror of the
/// sinking half's:
///
/// - both successors are distinct and each has the branching block as its
///   **only** predecessor, so no third path reaches the access;
/// - the two instructions are *identical* — same op, same operands, same slot;
/// - a loaded destination is written exactly twice in the whole function (by
///   these two instructions and nothing else) and every read of it is
///   dominated by one of the arms;
/// - operands are stable in the same sense the sinking half requires.
///
/// The move crosses only the branching block's terminator and the flag-setting
/// run in front of it, neither of which touches memory, so a load still reads
/// what it read and a store still writes what it wrote.
fn hoist(
    instructions: &mut Vec<CodeInstruction>,
    blocks: &[Block],
    overlay: &Ssa,
    models: &(ClassModel, ClassModel),
    where_of: &[usize],
    preds: &[Vec<usize>],
    def_count: &std::collections::HashMap<u32, usize>,
    uses_of: &std::collections::HashMap<u32, Vec<usize>>,
) -> u64 {
    // (insertion point in the branching block, the two sources to delete).
    let mut plans: Vec<(usize, usize, usize)> = Vec::new();
    for block in blocks {
        let terminator = block.end - 1;
        if !conditional_terminator(instructions[terminator].op) {
            continue;
        }
        if block.succ.len() < 2 || block.succ[0] == block.succ[1] {
            continue;
        }
        let (left, right) = (block.succ[0], block.succ[1]);
        if preds[left].len() != 1 || preds[right].len() != 1 {
            continue;
        }
        let (Some(first), Some(second)) = (
            leading_access(instructions, &blocks[left]),
            leading_access(instructions, &blocks[right]),
        ) else {
            continue;
        };
        if !same_instruction(&instructions[first], &instructions[second]) {
            continue;
        }
        if !operands_are_stable(instructions, &models.0, first, def_count) {
            continue;
        }
        // A loaded destination must be written by exactly these two and read
        // only where one of the arms dominates.
        if let Some(RegRef::VReg(destination)) =
            instructions[first].operand("dst").and_then(|operand| {
                crate::codegen::engine::regalloc::analysis::classify_ref(operand, &models.0)
            })
        {
            if def_count.get(&destination) != Some(&2) {
                continue;
            }
            let covered = uses_of.get(&destination).is_none_or(|uses| {
                uses.iter().all(|&use_index| {
                    overlay.dominates(left, where_of[use_index])
                        || overlay.dominates(right, where_of[use_index])
                })
            });
            if !covered {
                continue;
            }
        }
        let Some(point) = hoist_point(instructions, block) else {
            continue;
        };
        plans.push((point, first, second));
    }

    if plans.is_empty() {
        return 0;
    }
    let fired = plans.len() as u64;
    let mut removed = vec![false; instructions.len()];
    let mut arrivals: Vec<(usize, usize)> = Vec::new();
    for (point, first, second) in plans {
        removed[first] = true;
        removed[second] = true;
        // The surviving copy is the first arm's; the second is redundant.
        arrivals.push((point, first));
    }

    let taken = std::mem::take(instructions);
    let mut carried: Vec<Option<CodeInstruction>> = taken.into_iter().map(Some).collect();
    let mut rebuilt: Vec<CodeInstruction> = Vec::with_capacity(carried.len());
    for index in 0..carried.len() {
        for &(point, source) in &arrivals {
            if point == index {
                if let Some(instruction) = carried[source].take() {
                    rebuilt.push(instruction);
                }
            }
        }
        if removed[index] {
            continue;
        }
        if let Some(instruction) = carried[index].take() {
            rebuilt.push(instruction);
        }
    }
    *instructions = rebuilt;
    fired
}

/// The block's first real instruction, when it is an `sp`-slot access this row
/// may move (a leading label does not count).
fn leading_access(instructions: &[CodeInstruction], block: &Block) -> Option<usize> {
    let mut index = block.start;
    if index < block.end && instructions[index].op == CodeOp::Label {
        index += 1;
    }
    if index >= block.end {
        return None;
    }
    let instruction = &instructions[index];
    let memory = matches!(instruction.op, CodeOp::LdrU64 | CodeOp::StrU64) && sp_based(instruction);
    memory.then_some(index)
}

/// Whether two instructions are the same operation on the same operands.
fn same_instruction(a: &CodeInstruction, b: &CodeInstruction) -> bool {
    a.op == b.op
        && a.fields.len() == b.fields.len()
        && a.fields
            .iter()
            .zip(&b.fields)
            .all(|((an, av), (bn, bv))| an == bn && av.rendered() == bv.rendered())
}

/// Where in the branching block a hoisted access may land: before the
/// terminator, and before the flag-setting run in front of it, so nothing
/// lands between a comparison and the branch that reads it.
fn hoist_point(instructions: &[CodeInstruction], block: &Block) -> Option<usize> {
    let floor = if instructions[block.start].op == CodeOp::Label {
        block.start + 1
    } else {
        block.start
    };
    let mut point = block.end;
    if point > floor
        && crate::codegen::engine::regalloc::analysis::is_block_terminator(
            instructions[point - 1].op,
        )
    {
        point -= 1;
    }
    while point > floor && is_flag_setter(instructions[point - 1].op) {
        point -= 1;
    }
    (point >= floor).then_some(point)
}

/// The ops whose whole purpose is to leave a condition in the flags for the
/// branch that follows.
fn is_flag_setter(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::Cmp | CodeOp::CmpImm | CodeOp::Adds | CodeOp::Subs | CodeOp::FCmpD
    )
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

    fn ops(instructions: &[CodeInstruction]) -> Vec<CodeOp> {
        instructions.iter().map(|inst| inst.op).collect()
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
        with_opt_level(OptLevel(level), || sink(stream, &model));
    }

    /// A computation used only on the taken side moves there, so the
    /// fall-through stops paying for it.
    #[test]
    fn a_one_sided_computation_sinks_into_its_branch() {
        let mut stream = vec![
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::CmpImm,
                CodeOp::BranchEq,
                CodeOp::Ret,
                CodeOp::Label,
                CodeOp::Mul,
                CodeOp::Mov,
                CodeOp::Ret,
            ],
        );
    }

    /// A computation used on both sides stays where it is.
    #[test]
    fn a_two_sided_computation_stays() {
        let stream = || {
            vec![
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
                ci("b.eq", &[("target", "used")]),
                ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
                ci("ret", &[]),
                ci("label", &[("name", "used")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// A successor with a second predecessor could be reached without running
    /// `B`, so nothing sinks into it.
    #[test]
    fn a_shared_successor_is_not_a_sink_target() {
        let stream = || {
            vec![
                ci("b", &[("target", "used")]),
                ci("label", &[("name", "top")]),
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
                ci("b.eq", &[("target", "used")]),
                ci("ret", &[]),
                ci("label", &[("name", "used")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// A load sinks only when nothing between it and the branch may write
    /// memory.
    #[test]
    fn a_load_does_not_sink_past_a_call() {
        let stream = || {
            vec![
                ci(
                    "ldr_u64",
                    &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
                ),
                ci("bl", &[("target", "_helper")]),
                ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
                ci("b.eq", &[("target", "used")]),
                ci("ret", &[]),
                ci("label", &[("name", "used")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// With the path clear, the load does sink.
    #[test]
    fn a_load_sinks_when_the_path_is_memory_quiet() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[0].op, CodeOp::CmpImm);
        assert_eq!(stream[4].op, CodeOp::LdrU64);
    }

    /// A destination register written more than once is never moved: the
    /// overlay does not make the stream's registers single-assignment.
    #[test]
    fn a_multiply_defined_destination_stays() {
        let stream = || {
            vec![
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
                ci("b.eq", &[("target", "used")]),
                ci("mov", &[("dst", "%v3"), ("src", "%v1")]),
                ci("ret", &[]),
                ci("label", &[("name", "used")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// The same load leads both arms of a branch, so it belongs above the
    /// branch and the two copies go.
    #[test]
    fn an_identical_leading_load_is_hoisted() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
            ci("label", &[("name", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::LdrU64,
                CodeOp::CmpImm,
                CodeOp::BranchEq,
                CodeOp::Mov,
                CodeOp::Ret,
                CodeOp::Label,
                CodeOp::Mov,
                CodeOp::Ret,
            ],
            "one load above the branch, none in the arms"
        );
    }

    /// The hoist lands before the flag-setting compare, never between it and
    /// the branch that reads it.
    #[test]
    fn the_hoist_lands_before_the_compare() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
            ci("label", &[("name", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[0].op, CodeOp::LdrU64);
        assert_eq!(
            stream[1].op,
            CodeOp::CmpImm,
            "the compare still feeds its branch"
        );
        assert_eq!(stream[2].op, CodeOp::BranchEq);
    }

    /// Different slots are different accesses.
    #[test]
    fn a_different_slot_is_not_hoisted() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
                ci("b.eq", &[("target", "other")]),
                ci(
                    "ldr_u64",
                    &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
                ),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
                ci("label", &[("name", "other")]),
                ci(
                    "ldr_u64",
                    &[("dst", "%v3"), ("base", "sp"), ("offset", "16")],
                ),
                ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// An arm reachable from elsewhere would have the access removed from a
    /// path the branching block never ran, so the row declines.
    #[test]
    fn a_shared_arm_is_not_hoisted_from() {
        let stream = || {
            vec![
                ci("b", &[("target", "other")]),
                ci("label", &[("name", "top")]),
                ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
                ci("b.eq", &[("target", "other")]),
                ci(
                    "ldr_u64",
                    &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
                ),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
                ci("label", &[("name", "other")]),
                ci(
                    "ldr_u64",
                    &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
                ),
                ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// An identical store leading both arms is one store.
    #[test]
    fn an_identical_leading_store_is_hoisted() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "other")]),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
            ci("label", &[("name", "other")]),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::StrU64,
                CodeOp::CmpImm,
                CodeOp::BranchEq,
                CodeOp::Ret,
                CodeOp::Label,
                CodeOp::Ret,
            ],
        );
    }
    /// The rows are off below `-O3`.
    #[test]
    fn level_two_disables_the_rows() {
        let stream = || {
            vec![
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
                ci("b.eq", &[("target", "used")]),
                ci("ret", &[]),
                ci("label", &[("name", "used")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
