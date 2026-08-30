//! Def-use "assume dead, prove live" marking over a selected, pre-regalloc
//! instruction stream — the shared core of the two DCE rows. Register defs and
//! uses come from the allocator's own effect model
//! (`regalloc::analysis::effect` via `regalloc::class_models`), so the operand
//! vocabulary cannot drift from the machinery that colors the same stream.
//!
//! **Liveness seeds** are everything this analysis does not fully model: any
//! instruction outside the removable whitelist (stores, calls, returns,
//! labels, flag-setters — which is what structurally preserves MFB's checked
//! arithmetic: an `adds` and the error-raise code behind its `b.vs` are never
//! candidates), plus any whitelisted instruction whose destination is not a
//! virtual register (an ABI-role physical def is an effect). Conditional
//! branch terminators are the one configurable exception: plain DCE seeds them
//! live (control flow untouched); ADCE leaves them dead until some live
//! instruction proves control-dependent on them.
//!
//! Marking then propagates. With the [`super::ssa`] overlay supplied (both
//! DCE rows pass it), a live instruction's register uses make live exactly
//! the definitions that *actually reach* them — the use's SSA value's
//! defining instruction, or every argument-defining instruction through a
//! phi — so a definition of a vreg name no live use ever reads is removable
//! even when another definition of the same name is live. A use with no SSA
//! fact (an unreachable block) falls back to the conservative non-SSA edge:
//! every whitelisted definition of that vreg name is kept. When control
//! dependence is supplied, a live instruction also makes the conditional
//! terminators of its controlling blocks live.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{
    effect, is_block_terminator, is_unconditional_terminator, Block, ClassModel, RegRef,
};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegClass;

use super::postdom::PostDom;
use super::ssa::{Ssa, ValueDef};

/// The marking outcome: `live[i]` for every instruction. Anything not live is
/// removable by construction (whitelisted, vreg-dst, proven unused — or a
/// conditional branch nothing is control-dependent on).
pub(crate) struct Marking {
    pub(crate) live: Vec<bool>,
}

/// Pure, single-`dst`, flag-free, memory-free ALU ops — the only removal
/// candidates. Same family as the Opt2 constant folder plus the pure unary
/// bit ops; deliberately excludes flag-setters (`adds`/`subs`/compares),
/// loads/stores, address materialization, every FP op, and anything unlisted.
pub(crate) fn removable_op(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::Mov
            | CodeOp::MovImm
            | CodeOp::Add
            | CodeOp::AddImm
            | CodeOp::Sub
            | CodeOp::SubImm
            | CodeOp::Mul
            | CodeOp::SMulH
            | CodeOp::UMulH
            | CodeOp::And
            | CodeOp::Orr
            | CodeOp::Eor
            | CodeOp::Mvn
            | CodeOp::LslImm
            | CodeOp::LsrImm
            | CodeOp::AsrImm
            | CodeOp::Lslv
            | CodeOp::Lsrv
            | CodeOp::Asrv
            | CodeOp::Rorv
            | CodeOp::RorvW
            | CodeOp::Clz
            | CodeOp::Rbit
            | CodeOp::RevW
            | CodeOp::RevX
    )
}

/// Whether the instruction is a conditional block terminator (a flag- or
/// register-conditional branch): live only via control dependence under ADCE.
pub(crate) fn conditional_terminator(op: CodeOp) -> bool {
    is_block_terminator(op) && !is_unconditional_terminator(op)
}

/// Run the marking. `control` supplies blocks + control dependence for ADCE;
/// `None` is plain DCE (conditional branches seeded live, no block facts
/// needed). `ssa` supplies the per-use value resolution for precise marking;
/// without it (or for a use it has no fact for) every definition of a used
/// vreg is kept.
pub(crate) fn mark_live(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    control: Option<(&[Block], &PostDom)>,
    ssa: Option<&Ssa>,
) -> Marking {
    let n = instructions.len();
    // Which block each instruction belongs to (ADCE only).
    let block_of: Option<Vec<usize>> = control.map(|(blocks, _)| {
        let mut map = vec![0usize; n];
        for (index, block) in blocks.iter().enumerate() {
            for slot in &mut map[block.start..block.end] {
                *slot = index;
            }
        }
        map
    });

    // (class, id) -> whitelisted defining instructions. Non-whitelisted defs
    // are live from the start, so they never need lookup.
    let mut vreg_defs: HashMap<(RegClass, u32), Vec<usize>> = HashMap::new();
    let mut removable = vec![false; n];
    for (i, instruction) in instructions.iter().enumerate() {
        if !removable_op(instruction.op) {
            continue;
        }
        // Removable only when every def is a virtual register (a physical def
        // is an ABI effect). `effect` reports defs per class.
        let mut defs: Vec<(RegClass, u32)> = Vec::new();
        let mut phys_def = false;
        for model in [&models.0, &models.1] {
            for def in effect(instruction, model).defs {
                match def {
                    RegRef::VReg(id) => defs.push((model.class, id)),
                    RegRef::Phys(_) => phys_def = true,
                }
            }
        }
        if phys_def || defs.is_empty() {
            continue;
        }
        removable[i] = true;
        for def in defs {
            vreg_defs.entry(def).or_default().push(i);
        }
    }

    // Seed and propagate.
    let mut live = vec![false; n];
    let mut queue: Vec<usize> = Vec::new();
    for (i, instruction) in instructions.iter().enumerate() {
        let seeded = if removable[i] {
            false
        } else if conditional_terminator(instruction.op) && control.is_some() {
            false // ADCE: prove via control dependence
        } else {
            true
        };
        if seeded {
            live[i] = true;
            queue.push(i);
        }
    }
    // Memo for the SSA value walk: a value's contributors are marked once.
    let mut value_marked = vec![false; ssa.map_or(0, |ssa| ssa.values.len())];
    // Mark every instruction contributing to `value` live: its defining
    // instruction, or every phi argument's contributors, transitively (phi
    // cycles exist in loops, hence the memo).
    fn mark_value(
        value: usize,
        ssa: &Ssa,
        value_marked: &mut [bool],
        live: &mut [bool],
        queue: &mut Vec<usize>,
    ) {
        let mut values = vec![value];
        while let Some(v) = values.pop() {
            if value_marked[v] {
                continue;
            }
            value_marked[v] = true;
            match &ssa.values[v] {
                ValueDef::Inst(def) => {
                    if !live[*def] {
                        live[*def] = true;
                        queue.push(*def);
                    }
                }
                ValueDef::Phi { args, .. } => values.extend(args.iter().map(|&(_, v)| v)),
                ValueDef::Entry => {}
            }
        }
    }
    while let Some(i) = queue.pop() {
        // Data edges: with SSA facts, exactly the definitions reaching each
        // use; otherwise every whitelisted def of the vreg name.
        for model in [&models.0, &models.1] {
            for used in effect(&instructions[i], model).uses {
                if let RegRef::VReg(id) = used {
                    if let Some(value) = ssa.and_then(|ssa| ssa.value_of_use(i, (model.is_fp, id)))
                    {
                        mark_value(
                            value,
                            ssa.expect("value came from it"),
                            &mut value_marked,
                            &mut live,
                            &mut queue,
                        );
                        continue;
                    }
                    if let Some(defs) = vreg_defs.get(&(model.class, id)) {
                        for &def in defs {
                            if !live[def] {
                                live[def] = true;
                                queue.push(def);
                            }
                        }
                    }
                }
            }
        }
        // Control edges (ADCE): the conditional terminators of every block
        // this instruction's block is control-dependent on.
        if let (Some((blocks, postdom)), Some(block_of)) = (control, block_of.as_ref()) {
            for &controller in &postdom.controllers[block_of[i]] {
                let terminator = blocks[controller].end - 1;
                if !live[terminator] {
                    live[terminator] = true;
                    queue.push(terminator);
                }
            }
        }
    }
    Marking { live }
}
