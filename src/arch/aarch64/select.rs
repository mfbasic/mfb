//! AArch64 instruction selection (`MIR → machine ops`).
//!
//! The AArch64 tail of the MIR pipeline, consumed via `mir::Backend::select`.
//! Mirror ops map back to their one [`CodeOp`] over the identical field bag; a
//! fused flagless op expands back to the exact two instructions it folded — the
//! flag-setter (`cmp`/`fcmp`/`adds`/`subs`) and the flag-reading branch —
//! reproducing the stream the backend emits **byte-for-byte**, and `addr_of`
//! expands to the `adrp; add :lo12:` page pair. Lives here (not shared `mir.rs`)
//! so every backend's selection is symmetric, under its own `arch/<isa>/`.

use crate::arch::aarch64::abi;
use crate::arch::ops::CodeOp;
use crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER;
use crate::target::shared::code::mir::{
    fused_setter_codeop, rename_field_values, MirInstruction, MirOp, ARENA_BASE, FUSED_COND_FIELD,
    FUSED_SHARE_FIELD,
};
use crate::target::shared::code::CodeInstruction;

pub(crate) fn select_aarch64(instructions: &[MirInstruction]) -> Vec<CodeInstruction> {
    let mut out = Vec::with_capacity(instructions.len());
    for instruction in instructions {
        if instruction.op == MirOp::AddrOf {
            // Structural expand (plan-00-C): `addr_of <dst>, <sym>` → the exact
            // `adrp <dst>, <sym>; add_pageoff <dst>, <dst>, <sym>` pair the
            // builders emit today (`abi::load_page_address` + `add_page_offset`).
            let dst = instruction
                .fields
                .iter()
                .find(|(key, _)| *key == "dst")
                .map(|(_, value)| value.clone())
                .expect("addr_of carries a dst field");
            let symbol = instruction
                .fields
                .iter()
                .find(|(key, _)| *key == "symbol")
                .map(|(_, value)| value.clone())
                .expect("addr_of carries a symbol field");
            out.push(abi::load_page_address(&dst, &symbol));
            out.push(abi::add_page_offset(&dst, &dst, &symbol));
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            // Split the field bag at the `cond` marker: everything before it is
            // the flag-setter's operands; its value is the branch mnemonic;
            // everything after is the branch's operands (plus an optional
            // `share` marker).
            let split = instruction
                .fields
                .iter()
                .position(|(key, _)| *key == FUSED_COND_FIELD)
                .expect("fused MIR op carries a cond field");
            let setter_fields = instruction.fields[..split].to_vec();
            let branch_op = CodeOp::from_mnemonic(&instruction.fields[split].1)
                .expect("fused MIR op carries a valid branch mnemonic");
            let mut branch_fields = Vec::new();
            let mut shared = false;
            for (key, value) in &instruction.fields[split + 1..] {
                if *key == FUSED_SHARE_FIELD {
                    shared = true;
                } else {
                    branch_fields.push((*key, value.clone()));
                }
            }
            // A shared branch reuses the comparison the previous fused op already
            // emitted, so emit only its branch.
            if !shared {
                out.push(CodeInstruction {
                    op: setter_op,
                    fields: setter_fields,
                });
            }
            out.push(CodeInstruction {
                op: branch_op,
                fields: branch_fields,
            });
        } else {
            out.push(CodeInstruction {
                op: instruction
                    .op
                    .to_code()
                    .expect("non-fused MIR op maps to a single CodeOp"),
                fields: instruction.fields.clone(),
            });
        }
    }
    // Realize the plan-34-B role tokens (`%arg`/`%ret`/`%sysnr`/…) to their
    // AArch64 register spellings — the temporary Phase-3b seam that keeps the
    // encoder on today's `xN` input (byte-identical); Phase 4 deletes this and
    // realizes tokens directly. Then realize `arena_base` back to its pinned
    // register (plan-00-D §2, plan-34-A).
    for instruction in &mut out {
        for (_, value) in instruction.fields.iter_mut() {
            if let Some(reg) = abi::realize_abi_token(value) {
                *value = reg.to_string();
            }
        }
        rename_field_values(&mut instruction.fields, ARENA_BASE, ARENA_BASE_REGISTER);
    }
    out
}
