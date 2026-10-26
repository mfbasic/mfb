//! Load/store pair formation (plan-171-A §4.4) — the Level-1 machine row
//! `optimizations.md` "Load/store pair formation (ldp/stp)".
//!
//! Two directly adjacent 64-bit accesses of one kind through one base, 8 bytes
//! apart, become one AArch64 pair:
//!
//! ```text
//!   str x1, [x9, #16]          stp x1, x2, [x9, #16]
//!   str x2, [x9, #24]    ->
//!   ldr x3, [sp, #40]          ldp x4, x3, [sp, #32]
//!   ldr x4, [sp, #32]    ->
//! ```
//!
//! The pair performs exactly the two accesses the instructions did, at the
//! same addresses, with the same registers, so nothing a program computes can
//! change. The rules are the conditions under which that is true:
//!
//! * the two are adjacent in the stream — nothing between them, no label (a
//!   label is an instruction here, so a branch target never splits a pair) —
//!   and the pass never reorders, so it needs no alias analysis;
//! * one base register, the same spelling in both, and it is a register (not
//!   an unresolved stack-argument sentinel);
//! * the lower offset is a multiple of 8 within the pair's signed imm7 reach,
//!   −512…504 (the single forms reach 0…32760);
//! * for loads, the first load does not write the base (the second address
//!   would move) and the two destinations differ (the architecture leaves an
//!   `ldp` into one register twice UNPREDICTABLE — and the first load is dead);
//! * every operand field is one the pair carries (`src`/`dst`, `base`,
//!   `offset`), so no field is dropped.
//!
//! Faults are unchanged in kind: both halves address the same 16 bytes, and an
//! MFBASIC program has no handler that could observe that the first half
//! completed before the second faulted. A store-release/load-acquire is a
//! different op and never matches.
//!
//! The merged instruction keeps the first instruction's `source` (the compiler
//! call site, audit metadata only; plan-171-A Correction 2). It runs from
//! [`super::machine::finish`] at the end of `finalize_frame`, on final `sp`
//! offsets, after every other post-allocation pass.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;

/// The pair's imm7 reach for 64-bit registers, in bytes (`-64 * 8 ..= 63 * 8`).
const LOWEST: i64 = -512;
const HIGHEST: i64 = 504;

/// Merge every legal adjacent pair in `instructions`, left to right; returns how
/// many pairs it formed. A no-op below Level 1.
pub(crate) fn pair_adjacent_accesses(instructions: &mut Vec<CodeInstruction>) -> u64 {
    if !crate::optimizer::level_enabled(1) {
        return 0;
    }
    let mut formed = 0;
    let mut out = Vec::with_capacity(instructions.len());
    let mut stream = std::mem::take(instructions).into_iter().peekable();
    while let Some(first) = stream.next() {
        if let Some(pair) = stream.peek().and_then(|second| merge(&first, second)) {
            stream.next();
            out.push(pair);
            formed += 1;
        } else {
            out.push(first);
        }
    }
    *instructions = out;
    formed
}

/// The single pair `a` then `b` performs, if they may be merged.
fn merge(a: &CodeInstruction, b: &CodeInstruction) -> Option<CodeInstruction> {
    let (data, op) = match a.op {
        CodeOp::StrU64 => ("src", CodeOp::Stp),
        CodeOp::LdrU64 => ("dst", CodeOp::Ldp),
        _ => return None,
    };
    if b.op != a.op || !only_fields(a, data) || !only_fields(b, data) {
        return None;
    }
    let base = a.operand("base")?;
    let base_name = base.rendered();
    if b.operand("base")?.rendered() != base_name
        || base_name == abi::INCOMING_ARGS_SPELLING
        || base_name == abi::OUTGOING_ARGS_SPELLING
    {
        return None;
    }
    let (offset_a, offset_b) = (offset(a)?, offset(b)?);
    if (offset_a - offset_b).abs() != 8 {
        return None;
    }
    let lower = offset_a.min(offset_b);
    if !(LOWEST..=HIGHEST).contains(&lower) || lower % 8 != 0 {
        return None;
    }
    let (register_a, register_b) = (a.operand(data)?, b.operand(data)?);
    if is_stack_pointer(register_a) || is_stack_pointer(register_b) {
        return None;
    }
    if op == CodeOp::Ldp
        && (register_a.rendered() == base_name || register_a.rendered() == register_b.rendered())
    {
        return None;
    }
    let (low, high) = if offset_a < offset_b {
        (register_a, register_b)
    } else {
        (register_b, register_a)
    };
    let second = if op == CodeOp::Ldp { "dst2" } else { "src2" };
    let mut pair = super::rebuilt(
        op,
        vec![
            (data, low.clone()),
            (second, high.clone()),
            ("base", base.clone()),
            ("offset", Operand::from(lower.to_string().as_str())),
            ("addressing", Operand::from("offset")),
            ("width", Operand::from("8")),
        ],
    );
    pair.source = a.source;
    Some(pair)
}

/// Whether `instruction` carries exactly `data`, `base` and `offset`.
fn only_fields(instruction: &CodeInstruction, data: &str) -> bool {
    instruction.fields.len() == 3
        && instruction
            .fields
            .iter()
            .all(|(name, _)| *name == data || *name == "base" || *name == "offset")
}

fn offset(instruction: &CodeInstruction) -> Option<i64> {
    instruction.get("offset")?.parse().ok()
}

/// Register 31 is XZR in a pair's data fields, so `sp` can never be paired data.
fn is_stack_pointer(register: &Operand) -> bool {
    matches!(register.rendered().as_ref(), "sp" | "raw_sp")
}

#[cfg(test)]
mod tests;
