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
use crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER;
use crate::arch::ops::CodeOp;
use crate::target::shared::code::mir::{
    code_fields_from_mir, fused_setter_codeop, rename_operand_field_values, MirInstruction, MirOp,
    ARENA_BASE, FUSED_COND_FIELD, FUSED_SHARE_FIELD,
};
use crate::target::shared::code::{CodeInstruction, Operand};

pub(crate) fn select_aarch64(instructions: Vec<MirInstruction>) -> Vec<CodeInstruction> {
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
            out.push(abi::load_page_address(&dst, &symbol.render()));
            out.push(abi::add_page_offset(&dst, &dst, &symbol.render()));
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
            let setter_fields = code_fields_from_mir(&instruction.fields[..split]);
            let branch_op = CodeOp::from_mnemonic(&instruction.fields[split].1.render())
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
                    source: instruction.source,
                });
            }
            out.push(CodeInstruction {
                op: branch_op,
                fields: branch_fields,
                source: instruction.source,
            });
        } else {
            // Common (non-fused) case: MOVE the field bag into the CodeInstruction
            // instead of `code_fields_from_mir`'s `to_vec` clone (plan-84 Phase 2).
            let op = instruction
                .op
                .to_code()
                .expect("non-fused MIR op maps to a single CodeOp");
            let source = instruction.source;
            out.push(CodeInstruction {
                op,
                fields: instruction.fields,
                source,
            });
        }
    }
    // Realize the plan-34-B role tokens (`%arg`/`%ret`/`%sysnr`/…) to their
    // AArch64 register spellings, keeping the encoder on today's `xN` input
    // (byte-identical). This seam is permanent, not a Phase-3b stopgap: plan-34-B
    // Phase 4 tried to delete it and realize tokens directly, but that landed as
    // c098504f, broke every x86-64 program, and was reverted at a23aee06
    // (bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md leaves
    // the follow-up OPEN). Both other backends' selectors are built on the same
    // token→spelling seam, so it stays until a real neutral-stream redesign
    // replaces it. Then realize `arena_base` back to its pinned register
    // (plan-00-D §2, plan-34-A).
    for instruction in &mut out {
        for (_, value) in instruction.fields.iter_mut() {
            // Only a `Raw` `%`-token or a typed `Operand::Abi` can realize to a
            // register; a `VReg`/`Phys`/`Imm` never does. Match directly to skip
            // the per-field `render()` alloc over the whole selected stream
            // (plan-79).
            match value {
                Operand::Raw(text) => {
                    if let Some(reg) = abi::realize_abi_token(text) {
                        *value = Operand::from(reg);
                    }
                }
                // A convention-explicit ABI token (plan-85-A) realizes positionally
                // to `x{index}` — on AArch64 every convention/role collapses to the
                // same `xN`, so this is byte-identical to the legacy token.
                Operand::Abi { index, .. } => {
                    *value = Operand::from(abi::realize_abi_positional(*index));
                }
                _ => {}
            }
        }
        rename_operand_field_values(&mut instruction.fields, ARENA_BASE, ARENA_BASE_REGISTER);
    }
    // plan-85 Phase-D elision: the shared lowering now stages a libc/`%retC` result
    // into the aligned MFB result register (`mov return_register(),c_return(0)`)
    // after every C call. On AArch64 both realize to `x0`, so each is a `mov x0,x0`
    // no-op — remove them so this backend stays byte-identical to pre-plan-85 (the
    // real `mov rdi,rax` only exists on SysV-x86, where the banks split).
    crate::target::shared::code::elide_redundant_self_moves(&mut out);
    // plan-71-B Phase 1: the Category-2 self-move probe. `out` now carries fully
    // realized ABI registers (`%argK`/`%retK` → `xN`), so a same-index staging
    // move would already read as a `mov xN,xN` no-op here. Report those under
    // `MFB_BUG387_SELFMOVE`; unset, this reads nothing and every emitted byte is
    // unchanged (the scan never mutates `out`).
    if std::env::var_os("MFB_BUG387_SELFMOVE").is_some() {
        for line in crate::target::shared::code::bug387_selfmove_lines(&out, "aarch64") {
            eprintln!("{line}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::mir::lower_to_mir;

    fn values(out: &[CodeInstruction]) -> Vec<String> {
        out.iter()
            .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.render()))
            .collect()
    }

    #[test]
    fn explicit_abi_tokens_realize_to_positional_x_registers() {
        // plan-85-A: a typed `Operand::Abi` realizes positionally to `x{index}` on
        // AArch64 — every convention/role collapses to the same `xN`, so it is
        // byte-identical to the legacy `%argK`/`%retK`. %retC1 → x1, %argSys3 → x3,
        // %retSys → x0.
        let out = select_aarch64(lower_to_mir(&[
            CodeInstruction::new("mov")
                .field("dst", abi::mfb_return(0))
                .field("src", abi::mfb_arg(2)),
            CodeInstruction::new("mov")
                .field("dst", abi::c_return(1))
                .field("src", abi::sys_arg(3)),
            CodeInstruction::new("mov")
                .field("dst", abi::sys_return())
                .field("src", abi::c_arg(7)),
        ]));
        let vals = values(&out);
        assert!(vals.contains(&"x0".to_string()), "%retMFB0/%retSys → x0: {vals:?}");
        assert!(vals.contains(&"x2".to_string()), "%argMFB2 → x2: {vals:?}");
        assert!(vals.contains(&"x1".to_string()), "%retC1 → x1: {vals:?}");
        assert!(vals.contains(&"x3".to_string()), "%argSys3 → x3: {vals:?}");
        assert!(vals.contains(&"x7".to_string()), "%argC7 → x7: {vals:?}");
    }
}
