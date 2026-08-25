//! SSA overlay over the selected, pre-regalloc instruction stream — the Plan2
//! fact base the propagation rows and precise DCE marking consume.
//!
//! This is an **overlay**, not a rewrite: the stream itself keeps its `%vN` /
//! `%fN` virtual registers exactly as selection emitted them (the linear-scan
//! allocator colors that same stream afterwards), and SSA values exist only in
//! this analysis. Register defs and uses come from the allocator's own effect
//! model (`regalloc::analysis::effect` / `classify_ref` via
//! `regalloc::class_models`), and the CFG is the allocator's
//! (`analysis::build_cfg`), so the operand vocabulary and edges cannot drift
//! from the machinery that colors the same instructions.
//!
//! Construction is the classic Cytron pipeline run per function: forward
//! dominators (the Cooper–Harvey–Kennedy intersection algorithm — the same
//! code shape as [`super::postdom`], run on the forward graph from the entry
//! block), dominance frontiers, phi placement at the iterated dominance
//! frontier, and a dominator-tree renaming walk. Phis are placed semi-pruned
//! (Briggs): only for variables with an upward-exposed use in some block —
//! **plus every copy-source variable**, because the copy-forwarding facts
//! below query a variable's current value at points that are not its own
//! uses, and only full phi placement makes the renaming stack correct at
//! arbitrary dominated points, not just at uses. Block-local temporaries (the
//! overwhelming majority of a selected stream's vregs) get no phis and cost
//! only their local renaming.
//!
//! What consumers read:
//! - [`Ssa::value_of_use`]: the SSA value each `(instruction, variable)` use
//!   resolves to — [`ValueDef::Inst`] (a unique defining instruction),
//!   [`ValueDef::Phi`] (a join of values), or [`ValueDef::Entry`] (live-in /
//!   no def on the path). Precise DCE marks only the *actual* contributors of
//!   a live use instead of every definition of the vreg name; constant
//!   propagation evaluates a lattice over the values.
//! - [`Ssa::forwarded_source`]: for a use whose value was produced by a
//!   register-to-register copy (`mov` / `fmov_d_from_d`), the copy's
//!   *ultimate* source operand when that source provably still holds the same
//!   SSA value at the use point (checked against the renaming stacks during
//!   the walk, so a redefinition on any intervening path — which manifests as
//!   a phi — invalidates the forward). Copy propagation is then a pure field
//!   substitution.
//!
//! Instructions in blocks unreachable from the entry are never visited: their
//! uses have no facts (`value_of_use` returns `None`) and consumers fall back
//! to their conservative non-SSA behavior for them.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::{classify_ref, effect, Block, ClassModel, RegRef};
use crate::codegen::engine::types::CodeInstruction;

/// An SSA variable: `(is_fp, vreg id)` — the class-qualified virtual register
/// name. Physical registers are ABI effects, not SSA-tracked variables.
pub(crate) type Var = (bool, u32);

/// Index into [`Ssa::values`].
pub(crate) type ValueId = usize;

/// How one SSA value comes into being.
pub(crate) enum ValueDef {
    /// Defined by the instruction at this stream index.
    Inst(usize),
    /// A join: one value per (reachable) predecessor edge of the phi's block.
    Phi(Vec<ValueId>),
    /// Live-in at function entry, or no definition on the dominator path —
    /// an unknown the consumers must treat as opaque.
    Entry,
}

/// The overlay facts for one function's stream. See the module docs.
pub(crate) struct Ssa {
    /// Every SSA value, indexed by [`ValueId`].
    pub(crate) values: Vec<ValueDef>,
    /// `(instruction index, variable)` → the value that use reads. Absent for
    /// instructions in unreachable blocks.
    use_value: HashMap<(usize, Var), ValueId>,
    /// `(instruction index, variable)` → the copy-chain source operand this
    /// use may be rewritten to (copy propagation), valid at that point.
    copy_forward: HashMap<(usize, Var), Operand>,
}

impl Ssa {
    /// The SSA value the use of `var` at `inst` resolves to, when known.
    pub(crate) fn value_of_use(&self, inst: usize, var: Var) -> Option<ValueId> {
        self.use_value.get(&(inst, var)).copied()
    }

    /// The operand the use of `var` at `inst` may be rewritten to under copy
    /// propagation, when a forward is valid there.
    pub(crate) fn forwarded_source(&self, inst: usize, var: Var) -> Option<&Operand> {
        self.copy_forward.get(&(inst, var))
    }
}

/// A register-to-register copy this overlay chases: `mov` (integer class) or
/// `fmov_d_from_d` (FP class) with virtual dst *and* src of that class.
/// Returns `(dst var, src var, src operand)`.
fn copy_parts(
    instruction: &CodeInstruction,
    models: &(ClassModel, ClassModel),
) -> Option<(Var, Var, Operand)> {
    let model = match instruction.op {
        CodeOp::Mov => &models.0,
        CodeOp::FMovDFromD => &models.1,
        _ => return None,
    };
    let dst = classify_ref(instruction.operand("dst")?, model)?;
    let src_operand = instruction.operand("src")?;
    let src = classify_ref(src_operand, model)?;
    match (dst, src) {
        (RegRef::VReg(d), RegRef::VReg(s)) => {
            Some(((model.is_fp, d), (model.is_fp, s), src_operand.clone()))
        }
        _ => None,
    }
}

/// Build the overlay for one function. `blocks` must be
/// `analysis::build_cfg(instructions)` — the allocator's own CFG.
pub(crate) fn build(
    instructions: &[CodeInstruction],
    blocks: &[Block],
    models: &(ClassModel, ClassModel),
) -> Ssa {
    let empty = Ssa {
        values: Vec::new(),
        use_value: HashMap::new(),
        copy_forward: HashMap::new(),
    };
    let nb = blocks.len();
    if nb == 0 {
        return empty;
    }

    // Per-instruction def/use variable lists, from the allocator's own effect
    // model (both classes).
    let n = instructions.len();
    let mut inst_uses: Vec<Vec<Var>> = vec![Vec::new(); n];
    let mut inst_defs: Vec<Vec<Var>> = vec![Vec::new(); n];
    for (i, instruction) in instructions.iter().enumerate() {
        for model in [&models.0, &models.1] {
            let eff = effect(instruction, model);
            for used in eff.uses {
                if let RegRef::VReg(id) = used {
                    inst_uses[i].push((model.is_fp, id));
                }
            }
            for def in eff.defs {
                if let RegRef::VReg(id) = def {
                    inst_defs[i].push((model.is_fp, id));
                }
            }
        }
    }

    // Reverse postorder of the forward CFG from the entry block, and the
    // reachable set (a block outside it gets no facts at all).
    let mut order = Vec::with_capacity(nb); // postorder
    let mut seen = vec![false; nb];
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    seen[0] = true;
    while let Some(&mut (b, ref mut next)) = stack.last_mut() {
        if *next < blocks[b].succ.len() {
            let s = blocks[b].succ[*next];
            *next += 1;
            if !seen[s] {
                seen[s] = true;
                stack.push((s, 0));
            }
        } else {
            order.push(b);
            stack.pop();
        }
    }
    order.reverse();
    let mut rpo_index = vec![usize::MAX; nb];
    for (i, &b) in order.iter().enumerate() {
        rpo_index[b] = i;
    }
    // CFG predecessors (reachable edges only — everything below iterates
    // `order`, so an unreachable block never contributes or receives facts).
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for &b in &order {
        for &s in &blocks[b].succ {
            preds[s].push(b);
        }
    }

    // Forward dominators: Cooper–Harvey–Kennedy over the reverse postorder.
    let mut idom: Vec<Option<usize>> = vec![None; nb];
    idom[0] = Some(0);
    let intersect = |idom: &[Option<usize>], a: usize, b: usize| -> usize {
        let (mut a, mut b) = (a, b);
        while a != b {
            while rpo_index[a] > rpo_index[b] {
                a = idom[a].expect("processed in rpo order");
            }
            while rpo_index[b] > rpo_index[a] {
                b = idom[b].expect("processed in rpo order");
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &b in order.iter().skip(1) {
            let mut new_idom: Option<usize> = None;
            for &p in &preds[b] {
                if idom[p].is_none() {
                    continue;
                }
                new_idom = Some(match new_idom {
                    None => p,
                    Some(current) => intersect(&idom, current, p),
                });
            }
            if new_idom.is_some() && idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }

    // Dominator-tree children (root excluded) and dominance frontiers.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for &b in order.iter().skip(1) {
        children[idom[b].expect("reachable blocks are processed")].push(b);
    }
    // `visited_for[r] == b+1` marks that runner `r` already recorded join `b`
    // — dedup *and* early-stop, so shared dominator chains are not rewalked
    // (the classic linear DF construction; a `contains` scan here is
    // quadratic on the tens-of-thousands-block generated functions).
    let mut frontier: Vec<Vec<usize>> = vec![Vec::new(); nb];
    let mut visited_for: Vec<usize> = vec![0; nb];
    for &b in &order {
        if preds[b].len() < 2 {
            continue;
        }
        for &p in &preds[b] {
            let mut runner = p;
            while runner != idom[b].expect("reachable") && visited_for[runner] != b + 1 {
                visited_for[runner] = b + 1;
                frontier[runner].push(b);
                runner = idom[runner].expect("runner stays reachable");
            }
        }
    }

    // Variables needing phis (semi-pruned + copy sources; module docs), and
    // each one's def blocks.
    let mut needs_phis: HashMap<Var, Vec<usize>> = HashMap::new();
    let mut defined: HashSet<Var> = HashSet::new();
    for &b in &order {
        defined.clear();
        for i in blocks[b].start..blocks[b].end {
            for &var in &inst_uses[i] {
                if !defined.contains(&var) {
                    needs_phis.entry(var).or_default(); // upward-exposed use
                }
            }
            for &var in &inst_defs[i] {
                defined.insert(var);
            }
        }
    }
    // Every copy-source variable gets full phi placement too (module docs:
    // the forwarding validity check reads its stack at non-use points).
    // Scanning all instructions (including unreachable ones) only widens the
    // phi set — conservative in the right direction.
    for instruction in instructions {
        if let Some((_, src, _)) = copy_parts(instruction, models) {
            needs_phis.entry(src).or_default();
        }
    }
    for &b in &order {
        for i in blocks[b].start..blocks[b].end {
            for &var in &inst_defs[i] {
                if let Some(def_blocks) = needs_phis.get_mut(&var) {
                    // Blocks arrive in order, so a duplicate can only be the
                    // most recent entry — no scan needed.
                    if def_blocks.last() != Some(&b) {
                        def_blocks.push(b);
                    }
                }
            }
        }
    }

    // Phi placement at each variable's iterated dominance frontier.
    let mut values: Vec<ValueDef> = Vec::new();
    let mut phis: Vec<BTreeMap<Var, ValueId>> = vec![BTreeMap::new(); nb];
    // Per-variable epoch stamps replace per-variable `contains` scans (the
    // scans are quadratic on huge functions with many cross-block variables).
    let mut placed_stamp: Vec<usize> = vec![0; nb];
    let mut queued_stamp: Vec<usize> = vec![0; nb];
    for (epoch, (&var, def_blocks)) in needs_phis.iter().enumerate() {
        let epoch = epoch + 1; // 0 = never stamped
        let mut worklist: Vec<usize> = Vec::with_capacity(def_blocks.len());
        for &b in def_blocks {
            if queued_stamp[b] != epoch {
                queued_stamp[b] = epoch;
                worklist.push(b);
            }
        }
        while let Some(b) = worklist.pop() {
            for &f in &frontier[b] {
                if placed_stamp[f] == epoch {
                    continue;
                }
                placed_stamp[f] = epoch;
                let vid = values.len();
                values.push(ValueDef::Phi(Vec::new()));
                phis[f].insert(var, vid);
                if queued_stamp[f] != epoch {
                    queued_stamp[f] = epoch;
                    worklist.push(f); // a phi is itself a definition
                }
            }
        }
    }

    // Renaming: iterative dominator-tree preorder walk.
    let mut use_value: HashMap<(usize, Var), ValueId> = HashMap::new();
    let mut copy_forward: HashMap<(usize, Var), Operand> = HashMap::new();
    let mut stacks: HashMap<Var, Vec<ValueId>> = HashMap::new();
    let mut entry_values: HashMap<Var, ValueId> = HashMap::new();
    // value produced by a chased copy → (ultimate source var, the source's
    // value at the copy, the source operand spelling).
    let mut copy_source: HashMap<ValueId, (Var, ValueId, Operand)> = HashMap::new();
    let mut pushed: Vec<Var> = Vec::new();

    enum Frame {
        Enter(usize),
        Exit(usize),
    }
    let mut walk: Vec<Frame> = vec![Frame::Enter(0)];
    while let Some(frame) = walk.pop() {
        match frame {
            Frame::Exit(mark) => {
                while pushed.len() > mark {
                    let var = pushed.pop().expect("len checked");
                    stacks.get_mut(&var).expect("pushed implies stack").pop();
                }
            }
            Frame::Enter(b) => {
                let mark = pushed.len();

                // The block's phis define first.
                for (&var, &vid) in &phis[b] {
                    stacks.entry(var).or_default().push(vid);
                    pushed.push(var);
                }

                for i in blocks[b].start..blocks[b].end {
                    // Uses resolve against the pre-instruction stacks.
                    for &var in &inst_uses[i] {
                        let v = match stacks.get(&var).and_then(|s| s.last()) {
                            Some(&v) => v,
                            None => *entry_values.entry(var).or_insert_with(|| {
                                values.push(ValueDef::Entry);
                                values.len() - 1
                            }),
                        };
                        use_value.insert((i, var), v);
                        // Copy forwarding: valid only while the source still
                        // holds the same value here.
                        if let Some((src, at_copy, spelling)) = copy_source.get(&v) {
                            if stacks.get(src).and_then(|s| s.last()) == Some(at_copy) {
                                copy_forward.insert((i, var), spelling.clone());
                            }
                        }
                    }
                    // Defs push new values.
                    let copy = copy_parts(&instructions[i], models);
                    for &var in &inst_defs[i] {
                        let vid = values.len();
                        values.push(ValueDef::Inst(i));
                        stacks.entry(var).or_default().push(vid);
                        pushed.push(var);
                        if let Some((dst, src, spelling)) = &copy {
                            if *dst == var {
                                // The value the copy captured (resolved above,
                                // before this def pushed).
                                if let Some(&w) = use_value.get(&(i, *src)) {
                                    // Chain collapse: a copy of a still-valid
                                    // copy forwards to the original source.
                                    let record = match copy_source.get(&w) {
                                        Some((s0, w0, sp0))
                                            if stacks.get(s0).and_then(|s| s.last())
                                                == Some(w0) =>
                                        {
                                            (*s0, *w0, sp0.clone())
                                        }
                                        _ => (*src, w, spelling.clone()),
                                    };
                                    copy_source.insert(vid, record);
                                }
                            }
                        }
                    }
                }

                // Feed the successors' phis from this edge's stack tops.
                for &s in &blocks[b].succ {
                    for (&var, &phi_vid) in &phis[s] {
                        let arg = match stacks.get(&var).and_then(|s| s.last()) {
                            Some(&v) => v,
                            None => *entry_values.entry(var).or_insert_with(|| {
                                values.push(ValueDef::Entry);
                                values.len() - 1
                            }),
                        };
                        if let ValueDef::Phi(args) = &mut values[phi_vid] {
                            args.push(arg);
                        }
                    }
                }

                walk.push(Frame::Exit(mark));
                for &child in children[b].iter().rev() {
                    walk.push(Frame::Enter(child));
                }
            }
        }
    }

    Ssa {
        values,
        use_value,
        copy_forward,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::engine::regalloc;
    use crate::codegen::engine::regalloc::analysis::build_cfg;

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn overlay(instructions: &[CodeInstruction]) -> Ssa {
        let models = regalloc::class_models(&crate::arch::aarch64::regmodel::Aarch64RegisterModel);
        let blocks = build_cfg(instructions);
        build(instructions, &blocks, &models)
    }

    fn v(id: u32) -> Var {
        (false, id)
    }

    /// Straight-line redefinition: each use resolves to the *nearest* def, so
    /// the two uses of `%v1` see two distinct SSA values, each an `Inst` of
    /// the right index.
    #[test]
    fn uses_resolve_to_the_nearest_definition() {
        let stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
            ),
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "2")],
            ),
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        let ssa = overlay(&stream);
        let first = ssa.value_of_use(1, v(1)).expect("use at mov");
        let second = ssa.value_of_use(3, v(1)).expect("use at add");
        assert_ne!(first, second, "redefinition must split the values");
        assert!(matches!(ssa.values[first], ValueDef::Inst(0)));
        assert!(matches!(ssa.values[second], ValueDef::Inst(2)));
    }

    /// Diamond: `%v1` defined in both arms, used after the join — the use
    /// resolves to a phi whose args are the two arm definitions.
    #[test]
    fn join_use_resolves_to_a_phi_of_both_arms() {
        let stream = vec![
            /* 0 */ ci("b.eq", &[("target", "else")]),
            /* 1 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
            ),
            /* 2 */ ci("b", &[("target", "join")]),
            /* 3 */ ci("label", &[("name", "else")]),
            /* 4 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "2")],
            ),
            /* 5 */ ci("label", &[("name", "join")]),
            /* 6 */ ci("mov", &[("dst", "x0"), ("src", "%v1")]),
            /* 7 */ ci("ret", &[]),
        ];
        let ssa = overlay(&stream);
        let at_join = ssa.value_of_use(6, v(1)).expect("use after join");
        let ValueDef::Phi(args) = &ssa.values[at_join] else {
            panic!("join use must resolve to a phi");
        };
        assert_eq!(args.len(), 2);
        let mut sources: Vec<usize> = args
            .iter()
            .map(|&a| match ssa.values[a] {
                ValueDef::Inst(i) => i,
                _ => panic!("phi args must be the two arm defs"),
            })
            .collect();
        sources.sort_unstable();
        assert_eq!(sources, vec![1, 4]);
    }

    /// A use with no definition anywhere resolves to `Entry`, not to a panic
    /// or a bogus instruction.
    #[test]
    fn undefined_use_resolves_to_entry() {
        let stream = vec![ci("mov", &[("dst", "%v2"), ("src", "%v1")]), ci("ret", &[])];
        let ssa = overlay(&stream);
        let value = ssa.value_of_use(0, v(1)).expect("use fact");
        assert!(matches!(ssa.values[value], ValueDef::Entry));
    }

    /// Copy forwarding: a use of the copy's dst forwards to the source while
    /// the source is unchanged, and a copy-of-a-copy collapses to the
    /// original source.
    #[test]
    fn copy_chains_forward_to_the_ultimate_source() {
        let stream = vec![
            /* 0 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "7")],
            ),
            /* 1 */ ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            /* 2 */ ci("mov", &[("dst", "%v3"), ("src", "%v2")]),
            /* 3 */ ci("add", &[("dst", "%v4"), ("lhs", "%v3"), ("rhs", "%v2")]),
            /* 4 */ ci("ret", &[]),
        ];
        let ssa = overlay(&stream);
        let lhs = ssa.forwarded_source(3, v(3)).expect("chain forwards");
        assert_eq!(lhs.rendered(), "%v1", "copy-of-copy collapses to %v1");
        let rhs = ssa.forwarded_source(3, v(2)).expect("direct forward");
        assert_eq!(rhs.rendered(), "%v1");
    }

    /// A redefinition of the copy's source on an intervening path (which
    /// manifests as a phi at the join) invalidates the forward.
    #[test]
    fn source_redefinition_on_a_path_blocks_the_forward() {
        let stream = vec![
            /* 0 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "7")],
            ),
            /* 1 */ ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            /* 2 */ ci("b.eq", &[("target", "join")]),
            /* 3 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "8")],
            ),
            /* 4 */ ci("label", &[("name", "join")]),
            /* 5 */ ci("mov", &[("dst", "x0"), ("src", "%v2")]),
            /* 6 */ ci("ret", &[]),
        ];
        let ssa = overlay(&stream);
        assert!(
            ssa.forwarded_source(5, v(2)).is_none(),
            "%v1 may be 8 here; forwarding %v2 -> %v1 would be wrong"
        );
    }

    /// FP copies (`fmov_d_from_d`) forward within the FP class.
    #[test]
    fn fp_copies_forward_too() {
        let stream = vec![
            ci("fadd_d", &[("dst", "%f1"), ("lhs", "%f8"), ("rhs", "%f9")]),
            ci("fmov_d_from_d", &[("dst", "%f2"), ("src", "%f1")]),
            ci("fmul_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f2")]),
            ci("ret", &[]),
        ];
        let ssa = overlay(&stream);
        let fwd = ssa.forwarded_source(2, (true, 2)).expect("fp forward");
        assert_eq!(fwd.rendered(), "%f1");
    }
}
