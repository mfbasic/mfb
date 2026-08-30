//! Sparse conditional constant propagation (SCCP) — a Level-3 Opt2 catalog
//! row (`planning/optimizations.md`): the classic Wegman–Zadeck algorithm on
//! the SSA overlay ([`super::plans::ssa`]), propagating constants and
//! *reachability* together so each strengthens the other.
//!
//! The Level-2 [`super::constprop`] row is the pessimistic form: it proves a
//! value constant only from already-proven inputs, and a phi is constant only
//! when **every** incoming argument agrees — including arguments arriving on
//! edges that can never be taken. SCCP is optimistic: every value starts
//! `Top` ("no evidence yet") and every edge starts unreachable except the
//! entry's, then constants and reachable edges are discovered together until a
//! fixpoint. A phi meets only its **reachable** arguments, so the `x = 1` on
//! the sole live path survives a join whose other arm is dead; and a compare
//! of two proven constants decides its branch, marking one outgoing edge
//! unreachable and often making more phis constant. SCCP subsumes constprop's
//! *results*, but both keep their own `-v` counters, so a `-O3` build reports
//! what each contributed.
//!
//! Two rewrites land: an instruction whose result is a proven constant
//! becomes `mov_imm dst, <value>` (exactly [`super::constant_folding::fold_one`]'s
//! own verdict — the shared function both propagation rows evaluate through,
//! so trap discipline is structural: MFB's checked arithmetic lowers to
//! flag-setting ops `fold_one` refuses to model), and a conditional branch
//! whose outcome is decided becomes an unconditional `b` (taken) or is
//! deleted (not taken, control falls through). Flags are trusted only from a
//! `cmp`/`cmp_imm` whose operands are constant — the same rule and the same
//! [`super::branches::verdict`] table the Level-2 branch row uses, so the two
//! can never disagree — and never from `adds`/`subs`, so an `ErrOverflow`
//! guard is never folded. Blocks SCCP proves unreachable are left in place;
//! folding their guarding branch is what lets the unreachable-block row prune
//! them.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::{build_cfg, classify_ref, is_use_field, RegRef};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;
use crate::target::shared::regmodel::RegisterModel;

use super::branches::{bits, verdict};
use super::constant_folding::{fold_one, Step};
use super::plans::mark::{conditional_terminator, removable_op};
use super::plans::ssa::{self, ValueDef};

/// The constant lattice. `Top` = no evidence yet (the optimistic start),
/// `Const` = provably this value on every reachable path, `Bottom` = provably
/// not a single constant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lattice {
    Top,
    Const(u64),
    Bottom,
}

impl Lattice {
    /// The meet: `Top` yields to anything, equal constants agree, everything
    /// else falls to `Bottom`.
    fn meet(self, other: Lattice) -> Lattice {
        match (self, other) {
            (Lattice::Top, x) | (x, Lattice::Top) => x,
            (Lattice::Const(a), Lattice::Const(b)) if a == b => Lattice::Const(a),
            _ => Lattice::Bottom,
        }
    }
}

/// Run the SCCP row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        crate::optimizer::stats::count_sccp_rewrites(0);
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);
    let nb = blocks.len();

    let mut block_of = vec![0usize; instructions.len()];
    for (index, block) in blocks.iter().enumerate() {
        for slot in &mut block_of[block.start..block.end] {
            *slot = index;
        }
    }

    let mut lattice: Vec<Lattice> = vec![Lattice::Top; overlay.values.len()];
    // Live-in / no-reaching-definition values are opaque, and nothing ever
    // re-evaluates them — they must start at the bottom, not at the optimistic
    // `Top`. Leaving them `Top` makes every compare against a parameter
    // *defer forever*, so a loop's back edge is never marked reachable and its
    // counter folds to its first-iteration value: a miscompile the row's own
    // loop test catches.
    for (vid, def) in overlay.values.iter().enumerate() {
        if matches!(def, ValueDef::Entry) {
            lattice[vid] = Lattice::Bottom;
        }
    }
    // Reachable CFG edges as `(from, to)`; a block is reachable once some
    // incoming edge is, and block 0 is reachable by definition.
    let mut reachable_block = vec![false; nb];
    reachable_block[0] = true;
    // Per block, the predecessors whose edge into it is live — the phi meet's
    // membership test, kept as a short Vec because blocks have few
    // predecessors (a hashed edge set was measurably hot here).
    let mut live_pred: Vec<Vec<usize>> = vec![Vec::new(); nb];

    // Which values each value feeds, so a lowered value re-evaluates exactly
    // its consumers (the "sparse" in SCCP).
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); overlay.values.len()];
    for (vid, def) in overlay.values.iter().enumerate() {
        match def {
            ValueDef::Inst(i) => {
                for (name, operand) in &instructions[*i].fields {
                    if !is_use_field(name) {
                        continue;
                    }
                    if let Some(RegRef::VReg(id)) = classify_ref(operand, &models.0) {
                        if let Some(w) = overlay.value_of_use(*i, (false, id)) {
                            dependents[w].push(vid);
                        }
                    }
                }
            }
            ValueDef::Phi { args, .. } => {
                for &(_, arg) in args {
                    dependents[arg].push(vid);
                }
            }
            ValueDef::Entry => {}
        }
    }

    // Values defined by instructions in each block, so reaching a block for
    // the first time queues exactly its own values.
    let mut values_in_block: Vec<Vec<usize>> = vec![Vec::new(); nb];
    let mut phis_of_block: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for (vid, def) in overlay.values.iter().enumerate() {
        match def {
            ValueDef::Inst(i) => values_in_block[block_of[*i]].push(vid),
            ValueDef::Phi { block, .. } => phis_of_block[*block].push(vid),
            ValueDef::Entry => {}
        }
    }

    // The constant an instruction's register field resolves to right now.
    let field_const = |lattice: &[Lattice], i: usize, field: &str| -> Option<u64> {
        let operand = instructions[i].operand(field)?;
        match classify_ref(operand, &models.0)? {
            RegRef::VReg(id) => match lattice[overlay.value_of_use(i, (false, id))?] {
                Lattice::Const(value) => Some(value),
                Lattice::Top | Lattice::Bottom => None,
            },
            RegRef::Phys(_) => None,
        }
    };
    // Whether any register input of `i` is still `Top` (evaluation defers).
    let has_top_input = |lattice: &[Lattice], i: usize| -> bool {
        instructions[i].fields.iter().any(|(name, operand)| {
            is_use_field(name)
                && matches!(classify_ref(operand, &models.0), Some(RegRef::VReg(id))
                    if overlay
                        .value_of_use(i, (false, id))
                        .is_some_and(|w| lattice[w] == Lattice::Top))
        })
    };

    let evaluate = |vid: usize, lattice: &[Lattice], live_pred: &[Vec<usize>]| -> Lattice {
        match &overlay.values[vid] {
            // A live-in/opaque value is never a constant here.
            ValueDef::Entry => Lattice::Bottom,
            ValueDef::Inst(i) => {
                if has_top_input(lattice, *i) {
                    return Lattice::Top; // defer: evidence still incomplete
                }
                match fold_one(&instructions[*i], &|field| field_const(lattice, *i, field)) {
                    Step::Record(_, value) | Step::Replace(_, value) => Lattice::Const(value),
                    Step::KillDst | Step::Barrier => Lattice::Bottom,
                }
            }
            // The heart of SCCP: meet over *reachable* incoming edges only.
            ValueDef::Phi { block, args } => args
                .iter()
                .filter(|(pred, _)| live_pred[*block].contains(pred))
                .fold(Lattice::Top, |acc, &(_, arg)| acc.meet(lattice[arg])),
        }
    };

    // Per-block flag sources, resolved **once**: which values (or immediates)
    // the block's conditional terminator ultimately compares. Rescanning each
    // block's instructions on every fixpoint round instead made this row
    // quadratic — a 15x `-O3` regression on the giant generated functions,
    // caught by the browser stress build.
    #[derive(Clone, Copy)]
    enum FlagOperand {
        Value(usize),
        Const(u64),
        Opaque,
    }
    let operand_source = |i: usize, field: &str| -> FlagOperand {
        let Some(operand) = instructions[i].operand(field) else {
            return FlagOperand::Opaque;
        };
        match classify_ref(operand, &models.0) {
            Some(RegRef::VReg(id)) => match overlay.value_of_use(i, (false, id)) {
                Some(value) => FlagOperand::Value(value),
                None => FlagOperand::Opaque,
            },
            Some(RegRef::Phys(_)) => FlagOperand::Opaque,
            None => match bits(&operand.rendered()) {
                Some(value) => FlagOperand::Const(value),
                None => FlagOperand::Opaque,
            },
        }
    };
    let const_input = |lattice: &[Lattice], i: usize, field: &str| -> Option<u64> {
        match operand_source(i, field) {
            FlagOperand::Value(value) => match lattice[value] {
                Lattice::Const(constant) => Some(constant),
                _ => None,
            },
            FlagOperand::Const(_) | FlagOperand::Opaque => None,
        }
    };
    // `None` = unconditional edges, or flags that are not a modeled compare
    // (both edges always live).
    let mut flag_source: Vec<Option<(FlagOperand, FlagOperand)>> = vec![None; nb];
    // value -> blocks whose branch outcome depends on it.
    let mut branch_watch: Vec<Vec<usize>> = vec![Vec::new(); overlay.values.len()];
    for b in 0..nb {
        let block = &blocks[b];
        let terminator = &instructions[block.end - 1];
        if !conditional_terminator(terminator.op) || block.succ.len() < 2 {
            continue;
        }
        let mut source = None;
        for i in (block.start..block.end - 1).rev() {
            match instructions[i].op {
                CodeOp::Cmp | CodeOp::CmpImm => {
                    source = Some((operand_source(i, "lhs"), operand_source(i, "rhs")));
                    break;
                }
                op if removable_op(op) => {}
                // A flag-setter (`adds`, checked arithmetic) or anything
                // unmodeled: this branch is never decided here.
                _ => break,
            }
        }
        flag_source[b] = source;
        if let Some((lhs, rhs)) = source {
            for operand in [lhs, rhs] {
                if let FlagOperand::Value(value) = operand {
                    branch_watch[value].push(b);
                }
            }
        }
    }

    // The block's live outgoing edges under the current lattice. Separating
    // `Top` from `Bottom` is what makes the algorithm *conditional*: an
    // undecided-yet compare commits **no** edge (the block is revisited when
    // its condition settles), while a provably-variable one commits both.
    // Conflating them would commit the dead edge before the constant was
    // known and poison every join below it.
    let outgoing = |b: usize, lattice: &[Lattice]| -> Vec<usize> {
        let Some((lhs, rhs)) = flag_source[b] else {
            return blocks[b].succ.clone();
        };
        let state = |operand: FlagOperand| match operand {
            FlagOperand::Value(value) => lattice[value],
            FlagOperand::Const(value) => Lattice::Const(value),
            FlagOperand::Opaque => Lattice::Bottom,
        };
        let (a, c) = match (state(lhs), state(rhs)) {
            (Lattice::Top, _) | (_, Lattice::Top) => return Vec::new(), // defer
            (Lattice::Const(a), Lattice::Const(c)) => (a, c),
            _ => return blocks[b].succ.clone(),
        };
        match verdict(instructions[blocks[b].end - 1].op, a, c) {
            // succ = [branch target, fallthrough] (`analysis::build_cfg`).
            Some(true) => vec![blocks[b].succ[0]],
            Some(false) => vec![blocks[b].succ[1]],
            None => blocks[b].succ.clone(),
        }
    };

    // Worklist fixpoint over values and blocks — each value is re-evaluated
    // only when an input changes, each block only when its condition does.
    let mut value_work: Vec<usize> = values_in_block[0].clone();
    let mut block_work: Vec<usize> = vec![0];
    let mut visited_block = vec![false; nb];
    visited_block[0] = true;
    while !value_work.is_empty() || !block_work.is_empty() {
        while let Some(b) = block_work.pop() {
            for s in outgoing(b, &lattice) {
                if live_pred[s].contains(&b) {
                    continue;
                }
                live_pred[s].push(b);
                reachable_block[s] = true;
                // A new incoming edge re-meets every phi of the target block.
                value_work.extend(phis_of_block[s].iter().copied());
                if !visited_block[s] {
                    visited_block[s] = true;
                    value_work.extend(values_in_block[s].iter().copied());
                    block_work.push(s);
                }
            }
        }
        while let Some(vid) = value_work.pop() {
            if lattice[vid] == Lattice::Bottom {
                continue; // already at the bottom; it can only stay there
            }
            let new = evaluate(vid, &lattice, &live_pred);
            if new == lattice[vid] {
                continue;
            }
            lattice[vid] = new;
            value_work.extend(dependents[vid].iter().copied());
            for &b in &branch_watch[vid] {
                if visited_block[b] {
                    block_work.push(b);
                }
            }
        }
    }

    // Rewrites, decided first (the lattice closures borrow the stream) and
    // applied after. Instructions in blocks SCCP proved unreachable are left
    // alone — they never execute, and folding the guarding branch is what
    // exposes them to the unreachable-block row.
    enum Rewrite {
        Constant(String, u64),
        Unconditional(Operand),
        Delete,
    }
    let mut planned: Vec<(usize, Rewrite)> = Vec::new();
    for i in 0..instructions.len() {
        let b = block_of[i];
        if !reachable_block[b] {
            continue;
        }
        if conditional_terminator(instructions[i].op) && blocks[b].succ.len() >= 2 {
            let live = outgoing(b, &lattice);
            if live.len() == 1 {
                if live[0] == blocks[b].succ[0] {
                    let target = instructions[i]
                        .operand("target")
                        .cloned()
                        .expect("conditional branches carry a target");
                    planned.push((i, Rewrite::Unconditional(target)));
                } else {
                    planned.push((i, Rewrite::Delete));
                }
            }
            continue;
        }
        if instructions[i].op == CodeOp::MovImm {
            continue;
        }
        match fold_one(&instructions[i], &|field| const_input(&lattice, i, field)) {
            Step::Replace(dst, value) => planned.push((i, Rewrite::Constant(dst, value))),
            Step::Record(dst, value) if instructions[i].op == CodeOp::Mov => {
                planned.push((i, Rewrite::Constant(dst, value)))
            }
            _ => {}
        }
    }

    let fired = planned.len() as u64;
    let mut deleted: Vec<usize> = Vec::new();
    for (i, rewrite) in planned {
        match rewrite {
            Rewrite::Constant(dst, value) => {
                instructions[i] = abi::move_immediate(&dst, "Integer", &value.to_string());
            }
            Rewrite::Unconditional(target) => {
                instructions[i] = CodeInstruction::new("b").field("target", target);
            }
            Rewrite::Delete => deleted.push(i),
        }
    }
    if !deleted.is_empty() {
        let mut keep = vec![true; instructions.len()];
        for index in deleted {
            keep[index] = false;
        }
        let mut index = 0;
        instructions.retain(|_| {
            let keep = keep[index];
            index += 1;
            keep
        });
    }
    crate::optimizer::stats::count_sccp_rewrites(fired);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
        ci(
            "mov_imm",
            &[("dst", dst), ("type", "Integer"), ("value", value)],
        )
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        with_opt_level(OptLevel(level), || eliminate(stream, &Aarch64RegisterModel));
    }

    /// The row's reason to exist: the arm that assigns a *different* constant
    /// is unreachable (its guard is a constant compare), so the join is
    /// constant and the use after it folds — which the pessimistic Level-2
    /// constant-propagation row cannot prove.
    #[test]
    fn dead_arm_does_not_poison_the_join() {
        let mut stream = vec![
            /* 0 */ mov_imm("%v1", "1"),
            /* 1 */ ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "1")]),
            /* 2 */ ci("b.ne", &[("target", "other")]), // never taken: 1 == 1
            /* 3 */ mov_imm("%v2", "7"),
            /* 4 */ ci("b", &[("target", "join")]),
            /* 5 */ ci("label", &[("name", "other")]),
            /* 6 */ mov_imm("%v2", "9"),
            /* 7 */ ci("label", &[("name", "join")]),
            /* 8 */ ci("add_imm", &[("dst", "%v3"), ("src", "%v2"), ("imm", "1")]),
            /* 9 */
            ci(
                "str_u64",
                &[("src", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            /* 10 */ ci("ret", &[]),
        ];
        run(&mut stream, 3);
        // The never-taken branch folds away entirely...
        assert!(
            !stream.iter().any(|i| i.op == CodeOp::BranchNe),
            "the constant compare decides the branch"
        );
        // ...and 7 + 1 folds through the join the dead arm would have poisoned.
        let add = stream
            .iter()
            .find(|i| i.get("dst").as_deref() == Some("%v3"))
            .expect("the %v3 definition survives");
        assert_eq!(add.op, CodeOp::MovImm);
        assert_eq!(add.get("value").as_deref(), Some("8"));
    }

    /// A genuinely variable join stays variable: the branch is undecidable,
    /// both arms are reachable, and their disagreeing constants meet to
    /// Bottom.
    #[test]
    fn live_disagreeing_arms_stay_variable() {
        let mut stream = vec![
            ci("cmp", &[("lhs", "%v8"), ("rhs", "%v9")]),
            ci("b.ne", &[("target", "other")]),
            mov_imm("%v2", "7"),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "other")]),
            mov_imm("%v2", "9"),
            ci("label", &[("name", "join")]),
            ci("add_imm", &[("dst", "%v3"), ("src", "%v2"), ("imm", "1")]),
            ci(
                "str_u64",
                &[("src", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert!(stream.iter().any(|i| i.op == CodeOp::BranchNe));
        assert_eq!(stream[7].op, CodeOp::AddImm, "join is not constant");
    }

    /// Checked arithmetic's flags (`adds` → `b.vs`/`b.vc`) never decide a
    /// branch: the overflow guard and its raise path survive untouched.
    #[test]
    fn checked_arithmetic_guards_are_never_folded() {
        let mut stream = vec![
            mov_imm("%v1", "1"),
            mov_imm("%v2", "2"),
            ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.vc", &[("target", "ok")]),
            ci("bl", &[("target", "_raise")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
        ];
        let before: Vec<CodeOp> = stream.iter().map(|i| i.op).collect();
        run(&mut stream, 3);
        assert_eq!(stream.iter().map(|i| i.op).collect::<Vec<_>>(), before);
    }

    /// A loop-carried phi never settles on a constant (its back-edge argument
    /// depends on itself), so the optimistic start cannot leak a wrong value.
    #[test]
    fn loop_carried_values_do_not_become_constant() {
        let mut stream = vec![
            mov_imm("%v1", "0"),
            ci("label", &[("name", "head")]),
            ci("add_imm", &[("dst", "%v1"), ("src", "%v1"), ("imm", "1")]),
            ci("cmp", &[("lhs", "%v1"), ("rhs", "%v9")]),
            ci("b.ne", &[("target", "head")]),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[2].op, CodeOp::AddImm, "the counter stays variable");
        assert!(stream.iter().any(|i| i.op == CodeOp::BranchNe));
    }

    /// A compare against an opaque live-in value must resolve to "both edges
    /// live", never to "defer forever": live-in values start at the bottom of
    /// the lattice. (With them left optimistic, this loop's back edge was
    /// never marked reachable and the counter folded to its first-iteration
    /// value — the bug the loop test above catches from the other side.)
    #[test]
    fn live_in_compares_keep_both_edges() {
        let mut stream = vec![
            mov_imm("%v1", "3"),
            ci("cmp", &[("lhs", "%v1"), ("rhs", "%v9")]),
            ci("b.eq", &[("target", "equal")]),
            mov_imm("%v2", "1"),
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("label", &[("name", "equal")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            stream[2].op,
            CodeOp::BranchEq,
            "an opaque operand cannot decide the branch"
        );
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            mov_imm("%v1", "1"),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "1")]),
            ci("b.ne", &[("target", "other")]),
            ci("label", &[("name", "other")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::BranchNe);
    }
}
