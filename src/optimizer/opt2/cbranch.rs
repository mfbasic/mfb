//! Compare-with-zero branch formation (plan-171-B §4.3) — the Level-1 machine
//! row `optimizations.md` "Compare-with-zero branch (cbz/cbnz) formation".
//!
//! Every `IF`, result-tag check and runtime guard ends in "compare with zero,
//! branch". This row rewrites that pair, on the final selected stream, into
//! the target's shortest form:
//!
//! ```text
//!   AArch64   cmp x9, #0; b.eq L                 cbz  x9, L
//!             cmp x9, #0; b.lt L                 tbnz x9, #63, L
//!             mov x10, #2^k; and x11, x9, x10;   tbnz x9, #k, L
//!               cmp x11, #0; b.ne L
//!   x86-64    the same bit test                  test r, 1<<k / bt r, k; jcc
//!   riscv64   li t5, 2^k; and t6, t4, t5;        andi/slli t6, t4, …; bne/blt …
//!               rv.br ne t6, zero, L               (tbnz t4, #k, scratch t6)
//!   all       mov z, #0; cmp r, z  (z dead)      cmp r, #0   (rv.br r, zero)
//! ```
//!
//! **Why it is exact.** `cbz r` branches exactly when `cmp r, #0; b.eq` does,
//! `tbnz r, #63` exactly when `cmp r, #0; b.lt` does (the sign bit), and a bit
//! test exactly when `(r & 2^k) != 0`. What the compare-and-branch also did —
//! set the condition flags, and on a bit test write `t` (and `m`) — is what
//! the row must prove nobody observes:
//!
//! * the flags are dead after the branch on both edges
//!   (`plans::flagslive::dead_after`; riscv64 has none);
//! * every register an instruction it deletes wrote is dead after the branch
//!   (`plans::reglive`, which treats calls, returns and unknown code as reading
//!   everything), and nothing between that definition and the branch reads it;
//! * a register is known to hold a constant only from a `mov_imm` earlier in
//!   the same block, with nothing between that writes it, calls, or ends the
//!   block.
//!
//! A compare against a register known to hold zero is rewritten to compare
//! with the immediate (`cmp r, z` and `cmp r, #0` set identical flags;
//! `cmp z, r` only the same Z, so that operand order needs the flags dead and
//! an `eq`/`ne` branch); the `mov_imm` goes when its register is dead. x86-64
//! forms no `cbz` — it encodes `cmp_imm r, 0` as `test r, r` already — and
//! riscv64 needs none, its `rv.br` names `zero`.
//!
//! It runs from [`super::machine::finish_selected`], the last step of
//! `mir::route_function_through_mir`: the stream is final there — nothing
//! re-selects, re-allocates or re-frames it — so the register allocator never
//! meets the four branch ops (bug-746: a register-naming terminator), and the
//! analyses see the registers the encoder will. The analyses run once, before
//! any rewrite: every rewrite only deletes dead definitions and moves a read
//! of a register to a later point where it still holds the same value, so a
//! register or flag the first pass proved dead stays dead.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir::CompareBranchForms;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::{is_block_terminator, PhysMask};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::{flagslive, reglive};

/// How far back a `mov_imm` may sit from the instruction that relies on its
/// value. Bounds the scan in long straight-line blocks; the shapes the code
/// generators emit put it at most a few instructions back.
const CONSTANT_REACH: usize = 16;

/// What one function's run formed.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Formed {
    /// `cbz`/`cbnz`, and the `tbz`/`tbnz #63` sign tests (AArch64).
    pub(crate) zero_branches: u64,
    /// `tbz`/`tbnz` single-bit tests (every target).
    pub(crate) bit_tests: u64,
    /// Compares against a register known to hold zero, rewritten to compare
    /// with the immediate (riscv64: the `zero` register).
    pub(crate) zero_compares: u64,
}

/// Run the row over one final stream, in place.
pub(crate) fn form(
    instructions: &mut Vec<CodeInstruction>,
    forms: CompareBranchForms,
    model: &dyn RegisterModel,
    compiled: bool,
) -> Formed {
    if !instructions
        .iter()
        .any(|i| is_candidate_branch(i.op, forms))
    {
        return Formed::default();
    }
    let flags_dead = match forms {
        CompareBranchForms::Riscv64 => Vec::new(),
        _ => flagslive::dead_after(instructions),
    };
    let liveness = reglive::analyze(instructions, model, compiled);
    let mut row = Row {
        instructions,
        removed: Vec::new(),
        flags_dead,
        liveness,
        forms,
        formed: Formed::default(),
    };
    row.removed = vec![false; row.instructions.len()];
    for at in 0..row.instructions.len() {
        if !is_candidate_branch(row.instructions[at].op, forms) {
            continue;
        }
        match forms {
            CompareBranchForms::Riscv64 => row.riscv_branch(at),
            _ => row.flag_branch(at),
        }
    }
    let Row {
        instructions,
        removed,
        formed,
        ..
    } = row;
    let mut index = 0;
    instructions.retain(|_| {
        let keep = !removed[index];
        index += 1;
        keep
    });
    formed
}

fn is_candidate_branch(op: CodeOp, forms: CompareBranchForms) -> bool {
    match forms {
        CompareBranchForms::Riscv64 => op == CodeOp::RvBr,
        _ => matches!(
            op,
            CodeOp::BranchEq | CodeOp::BranchNe | CodeOp::BranchLt | CodeOp::BranchGe
        ),
    }
}

/// One run's state: the stream, what is deleted so far, and the facts proved
/// over the original stream.
struct Row<'a> {
    instructions: &'a mut Vec<CodeInstruction>,
    removed: Vec<bool>,
    flags_dead: Vec<bool>,
    liveness: reglive::FinalLiveness,
    forms: CompareBranchForms,
    formed: Formed,
}

impl Row<'_> {
    /// `cmp`/`cmp_imm` + `b.eq`/`b.ne`/`b.lt`/`b.ge` (AArch64, x86-64).
    fn flag_branch(&mut self, at: usize) {
        let Some(compare) = self.previous(at) else {
            return;
        };
        let condition = self.instructions[at].op;
        let flags_dead = self.flags_dead[at];
        let Some(register) = self.zero_compared(compare, condition, flags_dead) else {
            return;
        };
        if !flags_dead {
            return;
        }
        if matches!(condition, CodeOp::BranchEq | CodeOp::BranchNe)
            && self.bit_test(at, compare, &register)
        {
            return;
        }
        if self.forms != CompareBranchForms::Aarch64 {
            return;
        }
        let (op, bit) = match condition {
            CodeOp::BranchEq => (CodeOp::Cbz, None),
            CodeOp::BranchNe => (CodeOp::Cbnz, None),
            CodeOp::BranchLt => (CodeOp::Tbnz, Some(63)),
            _ => (CodeOp::Tbz, Some(63)),
        };
        self.replace_branch(at, op, register, bit, None);
        self.removed[compare] = true;
        self.formed.zero_branches += 1;
    }

    /// The register `compare` compares with zero, rewriting a compare against
    /// a register known to hold zero into `cmp_imm r, 0` (deleting that
    /// register's `mov_imm` when it is dead); `None` if it is no such compare.
    fn zero_compared(
        &mut self,
        compare: usize,
        condition: CodeOp,
        flags_dead: bool,
    ) -> Option<Operand> {
        let instruction = &self.instructions[compare];
        if !has_only(instruction, &["lhs", "rhs"]) {
            return None;
        }
        let lhs = instruction.operand("lhs")?.clone();
        let rhs = instruction.operand("rhs")?.clone();
        if instruction.op == CodeOp::CmpImm {
            return (rhs.rendered().parse::<i64>() == Ok(0) && is_tracked(&lhs)).then_some(lhs);
        }
        if instruction.op != CodeOp::Cmp {
            return None;
        }
        let (register, zero) = if is_zero_spelling(&rhs) && is_tracked(&lhs) {
            (lhs, None)
        } else if let Some(definition) = self.known_constant(compare, &rhs, 0) {
            (lhs, Some((rhs, definition)))
        } else if flags_dead
            && matches!(condition, CodeOp::BranchEq | CodeOp::BranchNe)
            && (is_zero_spelling(&lhs) || self.known_constant(compare, &lhs, 0).is_some())
        {
            // `cmp 0, r` sets Z exactly as `cmp r, 0` does, and nothing else
            // the same way — sound only for an equality branch with the flags
            // dead after it.
            let definition = self.known_constant(compare, &lhs, 0);
            (rhs, definition.map(|definition| (lhs, definition)))
        } else {
            return None;
        };
        if !is_tracked(&register) {
            return None;
        }
        let source = self.instructions[compare].source;
        self.instructions[compare] = super::rebuilt(
            CodeOp::CmpImm,
            vec![("lhs", register.clone()), ("rhs", Operand::from("0"))],
        );
        self.instructions[compare].source = source;
        self.formed.zero_compares += 1;
        if let Some((zero, definition)) = zero {
            self.drop_if_dead(definition, compare, &zero);
        }
        Some(register)
    }

    /// `mov_imm m, 2^k; and t, r, m; <compare t with zero>; <eq/ne branch>`
    /// becomes `tbz`/`tbnz r, #k` when `t` is dead after the branch. The flags
    /// (AArch64, x86-64) were proved dead by the caller.
    fn bit_test(&mut self, at: usize, compare: usize, tested: &Operand) -> bool {
        let Some(and) = self.previous(compare) else {
            return false;
        };
        let Some((register, bit, mask_definition, mask)) = self.masked(and, tested, at) else {
            return false;
        };
        let op = if self.instructions[at].op == CodeOp::BranchEq {
            CodeOp::Tbz
        } else {
            CodeOp::Tbnz
        };
        self.removed[and] = true;
        self.removed[compare] = true;
        self.replace_branch(at, op, register, Some(bit), None);
        self.drop_if_dead(mask_definition, at, &mask);
        self.formed.bit_tests += 1;
        true
    }

    /// `rv.br` (riscv64): a register known zero becomes `zero`, then
    /// `and t, r, m; rv.br eq/ne t, zero` becomes `tbz`/`tbnz r, #k` with `t`
    /// as the scratch.
    fn riscv_branch(&mut self, at: usize) {
        if !has_only(
            &self.instructions[at],
            &["lhs", "rhs", "cond", "target", "reason"],
        ) {
            return;
        }
        // Rewrite both operands first: one register may be on both sides, and
        // its `mov_imm` may go only once the branch reads it on neither.
        let mut zeroed = Vec::new();
        for side in ["lhs", "rhs"] {
            let Some(value) = self.instructions[at].operand(side).cloned() else {
                return;
            };
            if is_zero_spelling(&value) {
                continue;
            }
            if let Some(definition) = self.known_constant(at, &value, 0) {
                set_operand(&mut self.instructions[at], side, Operand::from("zero"));
                self.formed.zero_compares += 1;
                zeroed.push((definition, value));
            }
        }
        for (definition, value) in zeroed {
            self.drop_if_dead(definition, at, &value);
        }
        let branch = &self.instructions[at];
        let condition = branch.get("cond").unwrap_or_default();
        if condition != "eq" && condition != "ne" {
            return;
        }
        let (lhs, rhs) = (
            branch.operand("lhs").cloned(),
            branch.operand("rhs").cloned(),
        );
        let tested = match (lhs, rhs) {
            (Some(lhs), Some(rhs)) if is_zero_spelling(&rhs) && is_tracked(&lhs) => lhs,
            (Some(lhs), Some(rhs)) if is_zero_spelling(&lhs) && is_tracked(&rhs) => rhs,
            _ => return,
        };
        let Some(and) = self.previous(at) else {
            return;
        };
        let Some((register, bit, mask_definition, mask)) = self.masked(and, &tested, at) else {
            return;
        };
        let op = if condition == "eq" {
            CodeOp::Tbz
        } else {
            CodeOp::Tbnz
        };
        self.removed[and] = true;
        self.replace_branch(at, op, register, Some(bit), Some(tested));
        self.drop_if_dead(mask_definition, at, &mask);
        self.formed.bit_tests += 1;
    }

    /// For `and t, r, m` at `and` with `t` = `tested`, `m` known to hold 2^k
    /// and `t` dead after the branch at `at`: `(r, k, m's mov_imm, m)`.
    fn masked(
        &self,
        and: usize,
        tested: &Operand,
        at: usize,
    ) -> Option<(Operand, u32, usize, Operand)> {
        let instruction = &self.instructions[and];
        if instruction.op != CodeOp::And || !has_only(instruction, &["dst", "lhs", "rhs"]) {
            return None;
        }
        let dst = instruction.operand("dst")?;
        if !same_register(dst, tested) || self.live_after(at, tested) {
            return None;
        }
        let (lhs, rhs) = (instruction.operand("lhs")?, instruction.operand("rhs")?);
        for (mask, register) in [(rhs, lhs), (lhs, rhs)] {
            if !is_tracked(register) || same_register(mask, register) {
                continue;
            }
            if let Some((definition, value)) = self.constant_at(and, mask) {
                if value.is_power_of_two() {
                    return Some((
                        register.clone(),
                        value.trailing_zeros(),
                        definition,
                        mask.clone(),
                    ));
                }
            }
        }
        None
    }

    /// Rewrite the branch at `at` as `op register[, #bit][, scratch], target`.
    fn replace_branch(
        &mut self,
        at: usize,
        op: CodeOp,
        register: Operand,
        bit: Option<u32>,
        scratch: Option<Operand>,
    ) {
        let branch = &self.instructions[at];
        let target = branch
            .operand("target")
            .cloned()
            .expect("a conditional branch carries its target");
        let source = branch.source;
        // `reason` is the lowering's annotation of why it branches (`ifFalse`,
        // …); nothing reads it, but the dump keeps it.
        let reason = branch.operand("reason").cloned();
        let mut fields = vec![("register", register)];
        if let Some(bit) = bit {
            fields.push(("bit", Operand::from(bit.to_string().as_str())));
        }
        if let Some(scratch) = scratch {
            fields.push(("scratch", scratch));
        }
        fields.push(("target", target));
        if let Some(reason) = reason {
            fields.push(("reason", reason));
        }
        self.instructions[at] = super::rebuilt(op, fields);
        self.instructions[at].source = source;
    }

    /// Delete `definition` (a `mov_imm` of `register`) when `register` is dead
    /// after `at` and nothing between them still reads it.
    fn drop_if_dead(&mut self, definition: usize, at: usize, register: &Operand) {
        if self.live_after(at, register) {
            return;
        }
        let Some(bit) = mask_of(register) else {
            return;
        };
        let read_between = (definition + 1..at)
            .any(|index| !self.removed[index] && self.liveness.uses[index] & bit != 0);
        if !read_between {
            self.removed[definition] = true;
        }
    }

    /// Whether `register` may be read after instruction `at`.
    fn live_after(&self, at: usize, register: &Operand) -> bool {
        mask_of(register).is_none_or(|bit| self.liveness.live_out[at] & bit != 0)
    }

    /// The nearest earlier instruction still in the stream.
    fn previous(&self, at: usize) -> Option<usize> {
        (0..at).rev().find(|&index| !self.removed[index])
    }

    /// `Some(definition)` when `register` holds `value` at `at`.
    fn known_constant(&self, at: usize, register: &Operand, value: u64) -> Option<usize> {
        self.constant_at(at, register)
            .and_then(|(definition, held)| (held == value).then_some(definition))
    }

    /// The `mov_imm` that gave `register` its value at `at`, and that value,
    /// when it is in the same block within [`CONSTANT_REACH`] instructions and
    /// nothing between writes `register` or calls.
    fn constant_at(&self, at: usize, register: &Operand) -> Option<(usize, u64)> {
        let bit = mask_of(register)?;
        for index in (at.saturating_sub(CONSTANT_REACH)..at).rev() {
            if self.removed[index] {
                continue;
            }
            let instruction = &self.instructions[index];
            if self.liveness.opaque[index]
                || instruction.op == CodeOp::Label
                || is_block_terminator(instruction.op)
                || matches!(
                    instruction.op,
                    CodeOp::BranchLink | CodeOp::BranchLinkRegister | CodeOp::Svc
                )
            {
                return None;
            }
            if self.liveness.defs[index] & bit == 0 {
                continue;
            }
            if instruction.op != CodeOp::MovImm || !has_only(instruction, &["dst", "type", "value"])
            {
                return None;
            }
            // The immediate grammar the encoders read: a decimal (either sign),
            // or a Boolean `true`/`false` (1/0).
            let value = instruction.get("value")?;
            let value = match value.as_str() {
                "false" => 0,
                "true" => 1,
                text => text
                    .parse::<u64>()
                    .ok()
                    .or_else(|| text.parse::<i64>().ok().map(|v| v as u64))?,
            };
            return Some((index, value));
        }
        None
    }
}

/// Whether `instruction` carries exactly `allowed` fields (a `mov_imm` may omit
/// `type`) — so a rewrite drops nothing.
fn has_only(instruction: &CodeInstruction, allowed: &[&str]) -> bool {
    instruction
        .fields
        .iter()
        .all(|(name, _)| allowed.contains(name))
}

/// The zero register, which needs no constant proof.
fn is_zero_spelling(value: &Operand) -> bool {
    matches!(value.rendered().as_ref(), "xzr" | "zero")
}

/// A concrete integer register the analyses track (not `sp`, not zero).
fn is_tracked(value: &Operand) -> bool {
    !is_zero_spelling(value)
        && !matches!(value.rendered().as_ref(), "sp" | "rsp" | "raw_sp")
        && mask_of(value).is_some()
}

fn mask_of(value: &Operand) -> Option<PhysMask> {
    reglive::index_of(&value.rendered()).map(|index| 1 << index)
}

fn same_register(a: &Operand, b: &Operand) -> bool {
    matches!((mask_of(a), mask_of(b)), (Some(x), Some(y)) if x == y)
}

fn set_operand(instruction: &mut CodeInstruction, name: &str, value: Operand) {
    if let Some((_, slot)) = instruction.fields.iter_mut().find(|(key, _)| *key == name) {
        *slot = value;
    }
}

#[cfg(test)]
mod tests;
