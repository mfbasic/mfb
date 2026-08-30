//! Local value numbering — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): block-local redundancy elimination on the
//! selected pre-regalloc stream. A pure ALU instruction that recomputes an
//! expression already computed in the same block — same operation, operands
//! holding the same *values* (not merely the same names) — is rewritten to a
//! register copy of the earlier result; copy propagation then bypasses the
//! copy and DCE sweeps what strands.
//!
//! State is two tables: virtual-register name → abstract value number
//! (constants get shared numbers so equal literals compare equal), and
//! normalized expression → (result number, holding register). Commutative
//! operations sort their operand numbers, so `a + b` and `b + a` collide.
//! Everything runs in **virtual-register space only**: an instruction with a
//! physical-register operand is ineligible (a hidden clobber — an x86 `mul`
//! expansion, a call — could invalidate a physical holder invisibly, while
//! vregs are single-assignment *storage* the allocator preserves across
//! calls, so value numbers legitimately survive a `bl`). Register kills come
//! from the allocator's own effect model — any instruction's defs drop their
//! names' numbers, so a stale holder can never be reused — and a label (a
//! join) clears everything. Eligible rewrite sources are exactly the
//! [`removable_op`] whitelist minus `mov`/`mov_imm` (already the propagation
//! rows' domain): pure, flag-free, memory-free, so replacing the recompute
//! with a copy of the identical bits is behavior-preserving by construction,
//! and checked arithmetic (flag-setting) is never a candidate.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::{classify_ref, effect, ClassModel, RegRef};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::mark::removable_op;

/// One normalized expression-key part: an operand's value number, a known
/// Integer constant's bits (GVN canonicalizes `mov_imm`-defined values to
/// this so equal constants from different feeders match), or a literal
/// field's exact text (an immediate, a shift amount).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum KeyPart {
    Value(u64),
    Constant(u64),
    Literal(String),
}

/// Whether swapping the two register operands of `op` preserves its result.
pub(super) fn commutative(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::Add
            | CodeOp::Mul
            | CodeOp::SMulH
            | CodeOp::UMulH
            | CodeOp::And
            | CodeOp::Orr
            | CodeOp::Eor
    )
}

/// The value-numberable ops: the pure whitelist minus the moves (those are
/// the propagation rows' domain, and rewriting a `mov_imm` to a `mov` is no
/// improvement).
pub(super) fn numberable_op(op: CodeOp) -> bool {
    removable_op(op) && !matches!(op, CodeOp::Mov | CodeOp::MovImm)
}

/// Build a `mov dst, src` from the two operands.
pub(super) fn copy_of(dst: Operand, src: Operand) -> CodeInstruction {
    CodeInstruction::new("mov")
        .field("dst", dst)
        .field("src", src)
}

/// Run the LVN row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn eliminate(instructions: &mut [CodeInstruction], model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = crate::codegen::engine::regalloc::class_models(model);
    let int_model = &models.0;

    let mut next: u64 = 0;
    let mut fresh = || {
        next += 1;
        next
    };
    // Int-vreg spelling -> current value number.
    let mut numbers: HashMap<String, u64> = HashMap::new();
    // Integer literal text -> shared value number.
    let mut constants: HashMap<String, u64> = HashMap::new();
    // Normalized expression -> (result number, holder spelling, holder operand).
    let mut expressions: HashMap<(CodeOp, Vec<KeyPart>), (u64, String, Operand)> = HashMap::new();

    let mut fired = 0;
    for i in 0..instructions.len() {
        let instruction = &instructions[i];
        if instruction.op == CodeOp::Label {
            // A join: another path may have left different values everywhere.
            numbers.clear();
            constants.clear();
            expressions.clear();
            continue;
        }
        // The eligible-ALU arm: all-vreg/literal operands, single vreg dst.
        if numberable_op(instruction.op) {
            if let Some((key, dst_spelling, dst_operand)) =
                expression_key(instruction, &models, &mut numbers, &mut fresh)
            {
                if let Some((number, holder, holder_operand)) = expressions.get(&key) {
                    if numbers.get(holder) == Some(number) && *holder != dst_spelling {
                        let number = *number;
                        let replacement = copy_of(dst_operand, holder_operand.clone());
                        instructions[i] = replacement;
                        numbers.insert(dst_spelling, number);
                        fired += 1;
                        continue;
                    }
                }
                let number = fresh();
                numbers.insert(dst_spelling.clone(), number);
                expressions.insert(key, (number, dst_spelling, dst_operand));
                continue;
            }
            // Ineligible operands (a physical register): fall through to the
            // generic kill below.
        }
        let instruction = &instructions[i];
        match instruction.op {
            // A copy carries its source's number to the destination.
            CodeOp::Mov => {
                if let (Some(dst), Some(src)) = (
                    int_vreg_spelling(instruction, "dst", int_model),
                    int_vreg_spelling(instruction, "src", int_model),
                ) {
                    let number = *numbers.entry(src).or_insert_with(&mut fresh);
                    numbers.insert(dst, number);
                    continue;
                }
            }
            // Equal Integer literals share a number, so ALU over them dedups.
            CodeOp::MovImm
                if instruction.get("type").as_deref()
                    == Some(crate::target::shared::abi::IMMEDIATE_CLASS_INTEGER) =>
            {
                if let (Some(dst), Some(text)) = (
                    int_vreg_spelling(instruction, "dst", int_model),
                    instruction.get("value"),
                ) {
                    let number = *constants.entry(text).or_insert_with(&mut fresh);
                    numbers.insert(dst, number);
                    continue;
                }
            }
            _ => {}
        }
        // Generic arm: whatever this instruction defines no longer holds its
        // old value — the allocator's own effect model says what that is.
        // (Vreg values themselves survive calls: the allocator preserves
        // them, so no wider invalidation is needed for `bl`.)
        for def in effect(&instructions[i], int_model).defs {
            if let RegRef::VReg(id) = def {
                numbers.remove(&format!("%v{id}"));
            }
        }
    }
    crate::optimizer::stats::count_local_value_numberings(fired);
}

/// The instruction's normalized expression key plus its dst, when every
/// operand is an int vreg (numbered) or a non-register literal, and the dst
/// is a single int vreg.
fn expression_key(
    instruction: &CodeInstruction,
    models: &(ClassModel, ClassModel),
    numbers: &mut HashMap<String, u64>,
    fresh: &mut impl FnMut() -> u64,
) -> Option<((CodeOp, Vec<KeyPart>), String, Operand)> {
    let int_model = &models.0;
    let mut parts: Vec<KeyPart> = Vec::new();
    let mut dst: Option<(String, Operand)> = None;
    for (name, operand) in &instruction.fields {
        if *name == "dst" {
            match classify_ref(operand, int_model) {
                Some(RegRef::VReg(_)) => {
                    dst = Some((operand.rendered().into_owned(), operand.clone()))
                }
                _ => return None,
            }
            continue;
        }
        match classify_ref(operand, int_model) {
            Some(RegRef::VReg(_)) => {
                let spelling = operand.rendered().into_owned();
                let number = *numbers.entry(spelling).or_insert_with(&mut *fresh);
                parts.push(KeyPart::Value(number));
            }
            Some(RegRef::Phys(_)) => return None, // hidden clobbers could stale it
            None => {
                // Not an int register. An FP register (either kind) is not a
                // trackable input; anything else is a literal.
                if classify_ref(operand, &models.1).is_some() {
                    return None;
                }
                parts.push(KeyPart::Literal(operand.rendered().into_owned()));
            }
        }
    }
    let (dst_spelling, dst_operand) = dst?;
    if commutative(instruction.op) {
        parts.sort_by(|a, b| key_rank(a).cmp(&key_rank(b)));
    }
    Some(((instruction.op, parts), dst_spelling, dst_operand))
}

pub(super) fn key_rank(part: &KeyPart) -> (u8, u64, &str) {
    match part {
        KeyPart::Value(n) => (0, *n, ""),
        KeyPart::Constant(bits) => (1, *bits, ""),
        KeyPart::Literal(text) => (2, 0, text),
    }
}

fn int_vreg_spelling(
    instruction: &CodeInstruction,
    field: &str,
    int_model: &ClassModel,
) -> Option<String> {
    let operand = instruction.operand(field)?;
    match classify_ref(operand, int_model)? {
        RegRef::VReg(_) => Some(operand.rendered().into_owned()),
        RegRef::Phys(_) => None,
    }
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

    fn run(stream: &mut [CodeInstruction], level: u8) {
        with_opt_level(OptLevel(level), || eliminate(stream, &Aarch64RegisterModel));
    }

    /// The second identical computation becomes a copy — including the
    /// commuted spelling — and survives an intervening call (vreg values do).
    #[test]
    fn repeated_expressions_become_copies() {
        let mut stream = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("bl", &[("target", "_mfb_fn_callee")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v2"), ("rhs", "%v1")]),
            ci(
                "str_u64",
                &[("src", "%v4"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[2].op, CodeOp::Mov, "commuted recompute is a copy");
        assert_eq!(stream[2].get("src").as_deref(), Some("%v3"));
        assert_eq!(stream[2].get("dst").as_deref(), Some("%v4"));
    }

    /// A redefined *input* changes the expression's value: no reuse. A
    /// redefined *holder* invalidates the stored result: no reuse either.
    #[test]
    fn redefinitions_block_reuse() {
        let mut input_redefined = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut input_redefined, 3);
        assert_eq!(
            input_redefined[2].op,
            CodeOp::Add,
            "input changed: recompute"
        );

        let mut holder_redefined = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut holder_redefined, 3);
        assert_eq!(
            holder_redefined[2].op,
            CodeOp::Add,
            "holder stale: recompute"
        );
    }

    /// Equal constants share a number (dedup through `mov_imm` feeders), and
    /// a label clears all knowledge.
    #[test]
    fn constants_share_numbers_and_labels_clear() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "7")],
            ),
            ci(
                "mov_imm",
                &[("dst", "%v2"), ("type", "Integer"), ("value", "7")],
            ),
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v9")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v2"), ("rhs", "%v9")]),
            ci("label", &[("name", "join")]),
            ci("add", &[("dst", "%v5"), ("lhs", "%v1"), ("rhs", "%v9")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[3].op, CodeOp::Mov, "same constants, same expression");
        assert_eq!(stream[3].get("src").as_deref(), Some("%v3"));
        assert_eq!(stream[5].op, CodeOp::Add, "the label cleared the tables");
    }

    /// A physical-register operand makes the expression ineligible — hidden
    /// clobbers could invalidate it invisibly.
    #[test]
    fn physical_operands_are_ineligible() {
        let mut stream = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "x0"), ("rhs", "%v2")]),
            ci("add", &[("dst", "%v4"), ("lhs", "x0"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[1].op, CodeOp::Add);
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[1].op, CodeOp::Add);
    }
}
