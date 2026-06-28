//! Block-local store-to-load forwarding (plan-01-float-codegen Phase 3a).
//!
//! The bump-and-reset register model spills every value to a stack slot between
//! statements and defensively spills an operand to the stack before lowering the
//! next one (see `emit_float_binary` / `lower_comparison_binary`). That leaves
//! runs like
//!
//! ```text
//!   str x10, [sp, #0xa8]      ; spill a*b
//!   str x16, [sp, #0xb0]      ; spill c
//!   ldr x8,  [sp, #0xa8]      ; reload a*b  -- x10 still holds it
//!   ldr x9,  [sp, #0xb0]      ; reload c    -- x16 still holds it
//! ```
//!
//! This pass rewrites a load from a stack slot into a register move from the
//! register that last stored that slot, when that register provably still holds
//! the value. It never removes an instruction and never reorders, so it cannot
//! change behaviour; the win is turning a memory reload into a register move
//! (and exposing the source register to the hardware's own forwarding).
//!
//! Safety model. Forwarding is sound only while we know (a) which `sp` slot a
//! value was last stored to and (b) that the storing register has not been
//! overwritten since. We therefore track, per basic block, a map from `sp`
//! offset to the register that stored it, and:
//!
//!   * clear the whole map at any label, branch, call, return, `sp` adjustment,
//!     or any instruction whose register effects we do not explicitly model
//!     (the conservative default), and at any store to a non-`sp` base (which
//!     might alias a frame slot);
//!   * invalidate every slot whose source register is written by an instruction
//!     we do model.
//!
//! Because the safety of a *forward* depends only on the register-*write* model
//! (not on reads), mis-modelling a read can never produce a wrong forward; an
//! unmodelled op clears state instead.

use crate::arch::aarch64::{abi, ops::CodeOp};

use super::types::CodeInstruction;

/// What an instruction does to the registers that forwarding cares about.
enum Effect<'a> {
    /// Store `src` to `[sp + offset]`.
    StoreSp { src: &'a str, offset: &'a str },
    /// Load into `dst` from `[sp + offset]`.
    LoadSp { dst: &'a str, offset: &'a str },
    /// Defines exactly the single register named by its `dst` field; no other
    /// register or memory effect that matters here.
    DefDst,
    /// Reads/flags only — no register definition, no clobber (compares).
    NoDef,
    /// Anything else: control flow, calls, non-`sp` or sub-word stores, vector
    /// ops, `sp` adjustments, or any op we do not model. Forwarding state is
    /// flushed.
    Barrier,
}

fn classify(instruction: &CodeInstruction) -> Effect<'_> {
    match instruction.op {
        CodeOp::StrU64 => match (instruction.get("base"), instruction.get("offset"), instruction.get("src")) {
            (Some("sp"), Some(offset), Some(src)) => Effect::StoreSp { src, offset },
            _ => Effect::Barrier, // store to a non-sp base may alias a frame slot
        },
        CodeOp::LdrU64 => match (instruction.get("base"), instruction.get("offset"), instruction.get("dst")) {
            (Some("sp"), Some(offset), Some(dst)) => Effect::LoadSp { dst, offset },
            (Some(_), _, Some(_)) => Effect::DefDst, // non-sp load: just defines dst
            _ => Effect::Barrier,
        },
        // Compares write only the flags.
        CodeOp::Cmp | CodeOp::CmpImm | CodeOp::FCmpD | CodeOp::FCmpZeroD => Effect::NoDef,
        // Scalar ops that define exactly their single `dst` operand.
        CodeOp::Mov
        | CodeOp::MovImm
        | CodeOp::Add
        | CodeOp::Adds
        | CodeOp::Sub
        | CodeOp::Subs
        | CodeOp::And
        | CodeOp::Orr
        | CodeOp::Eor
        | CodeOp::Mvn
        | CodeOp::Mul
        | CodeOp::SMulH
        | CodeOp::UMulH
        | CodeOp::Adc
        | CodeOp::Rorv
        | CodeOp::RorvW
        | CodeOp::Lslv
        | CodeOp::Lsrv
        | CodeOp::Asrv
        | CodeOp::Clz
        | CodeOp::Rbit
        | CodeOp::RevW
        | CodeOp::RevX
        | CodeOp::SDiv
        | CodeOp::UDiv
        | CodeOp::MSub
        | CodeOp::LslImm
        | CodeOp::LsrImm
        | CodeOp::AsrImm
        | CodeOp::AddImm
        | CodeOp::SubImm
        | CodeOp::Adrp
        | CodeOp::AddPageOff
        | CodeOp::LdrU32
        | CodeOp::LdrU16
        | CodeOp::LdrU8
        | CodeOp::FMovXFromD
        | CodeOp::FMovDFromX
        | CodeOp::FAddD
        | CodeOp::FSubD
        | CodeOp::FMulD
        | CodeOp::FDivD
        | CodeOp::FNegD
        | CodeOp::FSqrtD
        | CodeOp::FMaddD
        | CodeOp::SCvtfDFromX
        | CodeOp::FCvtzsXFromD
        | CodeOp::FCvtmsXFromD
        | CodeOp::FCvtpsXFromD
        | CodeOp::FCvtasXFromD => {
            if instruction.get("dst").is_some() {
                Effect::DefDst
            } else {
                Effect::Barrier
            }
        }
        // Everything else (control flow, calls, sub-word/vector stores, vector
        // ops, sp adjustments, …) conservatively flushes state.
        _ => Effect::Barrier,
    }
}

/// Run store-to-load forwarding over one function's instruction stream, in
/// place. Must run before `finalize_frame` (offsets are still pre-prologue and
/// the callee-save area / `sp` adjustments are not yet present).
pub(super) fn forward_stores_to_loads(instructions: &mut [CodeInstruction]) {
    // slot offset -> register that last stored it (and still holds the value).
    let mut slots: Vec<(String, String)> = Vec::new();
    let invalidate_reg = |slots: &mut Vec<(String, String)>, reg: &str| {
        slots.retain(|(_, src)| src != reg);
    };
    let set_slot = |slots: &mut Vec<(String, String)>, offset: &str, src: &str| {
        if let Some(entry) = slots.iter_mut().find(|(off, _)| off == offset) {
            entry.1 = src.to_string();
        } else {
            slots.push((offset.to_string(), src.to_string()));
        }
    };
    let slot_reg = |slots: &[(String, String)], offset: &str| -> Option<String> {
        slots
            .iter()
            .find(|(off, _)| off == offset)
            .map(|(_, src)| src.clone())
    };

    for index in 0..instructions.len() {
        match classify(&instructions[index]) {
            Effect::StoreSp { src, offset } => {
                let (src, offset) = (src.to_string(), offset.to_string());
                set_slot(&mut slots, &offset, &src);
            }
            Effect::LoadSp { dst, offset } => {
                let (dst, offset) = (dst.to_string(), offset.to_string());
                if let Some(reg) = slot_reg(&slots, &offset) {
                    if reg != dst {
                        instructions[index] = abi::move_register(&dst, &reg);
                    }
                }
                // The load (or the rewritten move) defines `dst`.
                invalidate_reg(&mut slots, &dst);
            }
            Effect::DefDst => {
                if let Some(dst) = instructions[index].get("dst") {
                    let dst = dst.to_string();
                    invalidate_reg(&mut slots, &dst);
                } else {
                    slots.clear();
                }
            }
            Effect::NoDef => {}
            Effect::Barrier => slots.clear(),
        }
    }
}
