//! Integer-register liveness over a **final** stream (plan-171-B): the stream
//! after `mir::route_function_through_mir` selected it — allocated, framed,
//! every operand a concrete register of the target — where nothing will move
//! a register again.
//!
//! The allocator's own liveness (`regalloc::analysis`) answers a narrower
//! question: it never needs to know what a call or a `ret` reads, because a
//! virtual register is never live into a callee. A machine row that deletes
//! an instruction does: a value left in `x0` before `bl` is the callee's
//! argument. So every rule here leans live:
//!
//! * a register an instruction's fields name is read or written as the
//!   allocator's field roles say (`analysis::effect`, which also knows a
//!   write-back pair writes its base);
//! * a register-role field holding something that is neither a concrete
//!   integer register, an FP register, the stack pointer nor the zero register
//!   (an unrealized token, a typo) reads **every** register;
//! * a call to a compiled MFBASIC function (`_mfb_fn_*`, `_mfb_ifn_*`) reads
//!   every register outside the target's allocatable pool — its convention
//!   passes arguments in the argument bank and pinned registers only, and a
//!   pool register it read on entry would be an allocator bug — then destroys
//!   the caller-saved set; **any other call** (`bl` to a runtime helper or a C
//!   function, `blr`) and every `svc` read every register, because a
//!   hand-written helper may take its inputs anywhere;
//! * `ret` reads every register (results, callee-saved values) — except, in a
//!   compiled MFBASIC function, the allocatable caller-saved registers, which
//!   its callers treat as destroyed by the call (the allocator's clobber model
//!   for `_mfb_fn_*`); a branch out of the function or off its end leaves
//!   every register live, and `branch_self` reads none.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, call_clobber_mask, caller_saved_mask, effect, fp_physical_index,
    int_concrete_physical_index, is_block_terminator, is_register_field, ClassModel, PhysMask,
    RegRef,
};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::{RegClass, RegisterModel};
use std::collections::HashSet;

/// Every register: the conservative answer.
const ALL: PhysMask = PhysMask::MAX;

/// The integer register `name` names on a final stream, if it is one. `lr` is
/// AArch64's spelling of `x30` in the frame's save area (`ldp x25, lr, …`); no
/// other target spells a register that way.
pub(crate) fn index_of(name: &str) -> Option<u32> {
    match name {
        "lr" => Some(30),
        _ => int_concrete_physical_index(name),
    }
}

/// Spellings a register-role field may carry that name no integer register
/// this analysis tracks: the stack pointer and the zero register.
fn is_untracked_spelling(name: &str) -> bool {
    matches!(name, "sp" | "raw_sp" | "rsp" | "xzr" | "wzr" | "x31")
}

/// The analysis of one final stream, per instruction.
pub(crate) struct FinalLiveness {
    /// The integer registers instruction `i` writes (a call: those it destroys).
    pub(crate) defs: Vec<PhysMask>,
    /// The integer registers instruction `i` reads, under the rules above.
    pub(crate) uses: Vec<PhysMask>,
    /// The integer registers whose value at the point after instruction `i`
    /// may still be read.
    pub(crate) live_out: Vec<PhysMask>,
    /// Instruction `i` names a register this analysis cannot identify, so what
    /// it writes is unknown too: a constant-tracking scan must stop there.
    pub(crate) opaque: Vec<bool>,
}

/// Analyze `instructions`, a final stream of the target `model` describes.
///
/// `compiled` says the function is a compiled MFBASIC function (`_mfb_fn_*`,
/// `_mfb_ifn_*`), whose callers treat the caller-saved registers as destroyed
/// by the call: its `ret` then reads every register except the allocatable
/// caller-saved ones. Any other function's `ret` reads every register.
pub(crate) fn analyze(
    instructions: &[CodeInstruction],
    model: &dyn RegisterModel,
    compiled: bool,
) -> FinalLiveness {
    let class = ClassModel {
        class: RegClass::Int,
        parse_vreg: |_| None,
        physical_index: index_of,
        is_fp: false,
        caller_saved: caller_saved_mask(model, RegClass::Int, int_concrete_physical_index),
    };
    let pool = model
        .allocatable(RegClass::Int)
        .iter()
        .filter_map(|name| index_of(name))
        .fold(0, |mask: PhysMask, index| mask | (1 << index));
    let n = instructions.len();
    let mut defs = vec![0 as PhysMask; n];
    let mut uses = vec![0 as PhysMask; n];
    let mut opaque = vec![false; n];
    for (i, instruction) in instructions.iter().enumerate() {
        let (def, used, unknown) = masks(instruction, &class, pool, compiled);
        defs[i] = def;
        uses[i] = used;
        opaque[i] = unknown;
    }
    let blocks = build_cfg(instructions);
    let labels: HashSet<String> = instructions
        .iter()
        .filter(|instruction| instruction.op == CodeOp::Label)
        .filter_map(|instruction| instruction.get("name"))
        .collect();
    let leaves: Vec<bool> = blocks
        .iter()
        .map(|block| {
            let last = &instructions[block.end - 1];
            if !is_block_terminator(last.op) {
                return block.succ.is_empty();
            }
            !matches!(last.op, CodeOp::Ret | CodeOp::BranchSelf)
                && !last
                    .get("target")
                    .is_some_and(|target| labels.contains(&target))
        })
        .collect();
    let exit = |index: usize, live_in: &[PhysMask]| -> PhysMask {
        if leaves[index] {
            return ALL;
        }
        blocks[index]
            .succ
            .iter()
            .fold(0, |live, &s| live | live_in[s])
    };
    let mut live_in = vec![0 as PhysMask; blocks.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for index in (0..blocks.len()).rev() {
            let mut live = exit(index, &live_in);
            for i in (blocks[index].start..blocks[index].end).rev() {
                live = (live & !defs[i]) | uses[i];
            }
            if live != live_in[index] {
                live_in[index] = live;
                changed = true;
            }
        }
    }
    let mut live_out = vec![0 as PhysMask; n];
    for (index, block) in blocks.iter().enumerate() {
        let mut live = exit(index, &live_in);
        for i in (block.start..block.end).rev() {
            live_out[i] = live;
            live = (live & !defs[i]) | uses[i];
        }
    }
    FinalLiveness {
        defs,
        uses,
        live_out,
        opaque,
    }
}

/// The registers one instruction writes and reads, under the rules above, and
/// whether it names a register the analysis cannot identify.
fn masks(
    instruction: &CodeInstruction,
    class: &ClassModel,
    pool: PhysMask,
    compiled: bool,
) -> (PhysMask, PhysMask, bool) {
    let effect = effect(instruction, class);
    let bit = |reference: &RegRef| match *reference {
        RegRef::Phys(index) => 1 << index,
        RegRef::VReg(_) => ALL,
    };
    let mut def = effect.defs.iter().fold(0, |mask, r| mask | bit(r));
    let mut used = effect.uses.iter().fold(0, |mask, r| mask | bit(r));
    let unknown = instruction.fields.iter().any(|(name, value)| {
        // `cmp_imm`'s `rhs` is the one register-named field that holds an
        // immediate (`CodeInstruction::field`'s `is_register_role`).
        if !is_register_field(name) || (instruction.op == CodeOp::CmpImm && *name == "rhs") {
            return false;
        }
        let spelling = value.rendered();
        index_of(&spelling).is_none()
            && fp_physical_index(&spelling).is_none()
            && !is_untracked_spelling(&spelling)
    });
    if unknown {
        used = ALL;
    }
    match instruction.op {
        CodeOp::BranchLink => {
            def |= call_clobber_mask(instruction, class);
            let target = instruction.get("target").unwrap_or_default();
            used |= if target.starts_with("_mfb_fn_") || target.starts_with("_mfb_ifn_") {
                !pool
            } else {
                ALL
            };
        }
        CodeOp::Ret if compiled && !unknown => {
            used = !(pool & class.caller_saved);
        }
        CodeOp::BranchLinkRegister | CodeOp::Svc | CodeOp::Ret => {
            def |= call_clobber_mask(instruction, class);
            used = ALL;
        }
        _ => {}
    }
    (def, used, unknown)
}

#[cfg(test)]
mod tests;
