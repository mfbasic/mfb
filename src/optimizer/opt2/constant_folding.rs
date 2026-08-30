//! Constant folding (the Opt2 half) — the same Level-1 catalog row as
//! `opt1::constant_folding`, rerun on the selected MIR stream between
//! instruction selection and register allocation, where lowering-introduced
//! constant chains (address/size math, immediate shuffles) become visible.
//!
//! Block-local and register-tracked, in the mold of the post-regalloc
//! peepholes: a map from register name to the 64-bit value it provably holds,
//! fed only by `mov_imm` (Integer) and `mov`, consumed by the arch-neutral
//! single-`dst` ALU ops whose machine semantics are bit-identical on every
//! backend — wrapping 64-bit `add`/`sub`/`mul` (+`add_imm`/`sub_imm`), bitwise
//! `and`/`orr`/`eor`, and the immediate shifts. An instruction with all inputs
//! known becomes `mov_imm dst, <result>`; its feeding `mov_imm`s stay (no
//! liveness here — DCE is a different row) and regalloc/peepholes run on the
//! rewritten stream as usual.
//!
//! **Trap preservation for free:** MFB's checked user arithmetic lowers to the
//! flag-setting ops (`adds`/`subs`) plus explicit compare/branch/raise
//! sequences, none of which this pass models — so a would-trap computation is
//! never folded here, only the raw, wrapping machine ops lowering itself
//! emits. Soundness follows the peephole model: any instruction not explicitly
//! modeled clears the whole map (a label is a join point; a call, store, or
//! flag op might do anything), so mis-modeling can only lose folds, never
//! invent one. `mul` with unknown inputs also clears everything rather than
//! killing only its `dst`: on x86-64 it expands to an rdx:rax-clobbering
//! sequence (bug-284 C8) and this pass is backend-agnostic.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;

/// Run the MIR constant folder over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (1); the fold count feeds the same
/// "Constant folding" `mfb build -v` line as the Opt1 half.
pub(crate) fn fold_constants(instructions: &mut [CodeInstruction]) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let mut known: HashMap<String, u64> = HashMap::new();
    let mut folded = 0;
    for instruction in instructions.iter_mut() {
        let step = fold_one(instruction, &|field| {
            instruction
                .get(field)
                .and_then(|name| known.get(&name).copied())
        });
        match step {
            Step::Record(dst, value) => {
                known.insert(dst, value);
            }
            Step::Replace(dst, value) => {
                *instruction = abi::move_immediate(&dst, "Integer", &value.to_string());
                known.insert(dst, value);
                folded += 1;
            }
            Step::KillDst => {
                if let Some(dst) = instruction.get("dst") {
                    known.remove(&dst);
                } else {
                    known.clear();
                }
            }
            Step::Barrier => known.clear(),
        }
    }
    crate::optimizer::stats::count_constant_folds(folded);
}

/// What one instruction does to the known-constant map. pub(super): the SSA
/// constant-propagation row (`opt2::constprop`) evaluates the same fold
/// semantics per SSA value, so the two rows cannot drift.
pub(super) enum Step {
    /// `dst` now provably holds the value; instruction unchanged.
    Record(String, u64),
    /// All inputs known: rewrite the instruction to `mov_imm dst, value`.
    Replace(String, u64),
    /// Defines exactly `dst` with an unknown value.
    KillDst,
    /// Unmodeled (label/branch/call/store/flags/…): forget everything.
    Barrier,
}

/// Evaluate one instruction against `resolve_reg`, which supplies the known
/// 64-bit value of the register named by an operand *field* (block-local:
/// the name→value map; SSA: the use's value's constant). The fold rules and
/// their soundness argument live in the module docs.
pub(super) fn fold_one(
    instruction: &CodeInstruction,
    resolve_reg: &dyn Fn(&str) -> Option<u64>,
) -> Step {
    // A register operand's known value, or a literal field's bits.
    let reg = |field: &str| -> Option<u64> { resolve_reg(field) };
    let imm = |field: &str| -> Option<u64> { instruction.get(field).and_then(|text| bits(&text)) };
    let dst = || instruction.get("dst");

    let folded = match instruction.op {
        CodeOp::MovImm => {
            // Only Integer immediates enter the domain (an FP payload's bits
            // don't feed GPR arithmetic).
            if instruction.get("type").as_deref()
                == Some(crate::target::shared::abi::IMMEDIATE_CLASS_INTEGER)
            {
                match (dst(), imm("value")) {
                    (Some(dst), Some(value)) => return Step::Record(dst, value),
                    _ => return Step::KillDst,
                }
            }
            return Step::KillDst;
        }
        CodeOp::Mov => {
            return match (dst(), reg("src")) {
                (Some(dst), Some(value)) => Step::Record(dst, value),
                _ => Step::KillDst,
            };
        }
        // Wrapping 64-bit ALU — bit-identical semantics on every backend.
        CodeOp::Add => binary(reg("lhs"), reg("rhs"), u64::wrapping_add),
        CodeOp::Sub => binary(reg("lhs"), reg("rhs"), u64::wrapping_sub),
        CodeOp::And => binary(reg("lhs"), reg("rhs"), |a, b| a & b),
        CodeOp::Orr => binary(reg("lhs"), reg("rhs"), |a, b| a | b),
        CodeOp::Eor => binary(reg("lhs"), reg("rhs"), |a, b| a ^ b),
        CodeOp::AddImm => binary(reg("src"), imm("imm"), u64::wrapping_add),
        CodeOp::SubImm => binary(reg("src"), imm("imm"), u64::wrapping_sub),
        // Immediate shifts: the constructors emit 0..=63, but guard anyway —
        // a 64-bit shift amount is ISA-divergent, so it must never fold.
        CodeOp::LslImm => shift(reg("src"), imm("shift"), |a, s| a << s),
        CodeOp::LsrImm => shift(reg("src"), imm("shift"), |a, s| a >> s),
        CodeOp::AsrImm => shift(reg("src"), imm("shift"), |a, s| ((a as i64) >> s) as u64),
        CodeOp::Mul => {
            return match (dst(), binary(reg("lhs"), reg("rhs"), u64::wrapping_mul)) {
                (Some(dst), Some(value)) => Step::Replace(dst, value),
                // Unknown-input mul stays, and on x86-64 its expansion clobbers
                // rdx:rax beyond `dst` (bug-284 C8) — drop everything.
                _ => Step::Barrier,
            };
        }
        _ => return Step::Barrier,
    };
    match (dst(), folded) {
        (Some(dst), Some(value)) => Step::Replace(dst, value),
        _ => Step::KillDst,
    }
}

fn binary(a: Option<u64>, b: Option<u64>, apply: impl Fn(u64, u64) -> u64) -> Option<u64> {
    Some(apply(a?, b?))
}

fn shift(a: Option<u64>, amount: Option<u64>, apply: impl Fn(u64, u32) -> u64) -> Option<u64> {
    let (a, amount) = (a?, amount?);
    (amount < 64).then(|| apply(a, amount as u32))
}

/// A literal field's 64-bit pattern: unsigned decimal (the emitters' form —
/// `u64::MAX` spells `-1`) or signed decimal, else not a constant.
fn bits(text: &str) -> Option<u64> {
    text.parse::<u64>()
        .ok()
        .or_else(|| text.parse::<i64>().ok().map(|signed| signed as u64))
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

    fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
        ci(
            "mov_imm",
            &[("dst", dst), ("type", "Integer"), ("value", value)],
        )
    }

    fn render(instructions: &[CodeInstruction]) -> Vec<String> {
        instructions
            .iter()
            .map(|inst| {
                let fields: Vec<String> = inst
                    .fields
                    .iter()
                    .map(|(name, _)| format!("{name}={}", inst.get(name).unwrap_or_default()))
                    .collect();
                format!("{:?} {}", inst.op, fields.join(" "))
            })
            .collect()
    }

    #[test]
    fn folds_known_alu_chains_to_mov_imm() {
        let mut stream = vec![
            mov_imm("%1", "2"),
            mov_imm("%2", "3"),
            ci("add", &[("dst", "%3"), ("lhs", "%1"), ("rhs", "%2")]),
            ci("mov", &[("dst", "%4"), ("src", "%3")]),
            ci("add_imm", &[("dst", "%5"), ("src", "%4"), ("imm", "10")]),
            ci("lsl_imm", &[("dst", "%6"), ("src", "%5"), ("shift", "2")]),
            ci("mul", &[("dst", "%7"), ("lhs", "%6"), ("rhs", "%2")]),
        ];
        with_opt_level(OptLevel(1), || fold_constants(&mut stream));
        // 2+3=5; mov propagates; 5+10=15; 15<<2=60; 60*3=180.
        assert_eq!(render(&stream)[2], "MovImm dst=%3 type=Integer value=5");
        assert_eq!(render(&stream)[4], "MovImm dst=%5 type=Integer value=15");
        assert_eq!(render(&stream)[5], "MovImm dst=%6 type=Integer value=60");
        assert_eq!(render(&stream)[6], "MovImm dst=%7 type=Integer value=180");
    }

    /// A label is a join point — another path may leave different values in
    /// the same registers, so knowledge must not cross it.
    #[test]
    fn labels_and_unmodeled_ops_clear_knowledge() {
        let mut stream = vec![
            mov_imm("%1", "2"),
            ci("label", &[("name", "join")]),
            ci("add", &[("dst", "%2"), ("lhs", "%1"), ("rhs", "%1")]),
        ];
        with_opt_level(OptLevel(1), || fold_constants(&mut stream));
        assert_eq!(stream[2].op, CodeOp::Add, "fold must not cross a label");
    }

    /// Unknown-input arithmetic kills its dst; a later use of that dst must
    /// not fold from the stale constant.
    #[test]
    fn redefinition_kills_the_constant() {
        let mut stream = vec![
            mov_imm("%1", "2"),
            ci("add", &[("dst", "%1"), ("lhs", "%1"), ("rhs", "%9")]),
            ci("add", &[("dst", "%2"), ("lhs", "%1"), ("rhs", "%1")]),
        ];
        with_opt_level(OptLevel(1), || fold_constants(&mut stream));
        assert_eq!(stream[1].op, CodeOp::Add);
        assert_eq!(stream[2].op, CodeOp::Add, "stale constant must not fold");
    }

    /// Wrapping is the machine semantic being folded (checked user arithmetic
    /// lowers to flag-setting ops this pass never models).
    #[test]
    fn folds_with_wrapping_machine_semantics() {
        let mut stream = vec![
            mov_imm("%1", &u64::MAX.to_string()),
            mov_imm("%2", "1"),
            ci("add", &[("dst", "%3"), ("lhs", "%1"), ("rhs", "%2")]),
        ];
        with_opt_level(OptLevel(1), || fold_constants(&mut stream));
        assert_eq!(render(&stream)[2], "MovImm dst=%3 type=Integer value=0");
    }

    #[test]
    fn level_zero_disables_the_fold() {
        let stream = || {
            vec![
                mov_imm("%1", "2"),
                mov_imm("%2", "3"),
                ci("add", &[("dst", "%3"), ("lhs", "%1"), ("rhs", "%2")]),
            ]
        };
        let mut off = stream();
        with_opt_level(OptLevel(0), || fold_constants(&mut off));
        assert_eq!(off[2].op, CodeOp::Add, "-O0 must not fold");

        let mut on = stream();
        with_opt_level(OptLevel(1), || fold_constants(&mut on));
        assert_ne!(on[2].op, CodeOp::Add, "-O1 must fold");
    }
}
