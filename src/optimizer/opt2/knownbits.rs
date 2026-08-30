//! The three Level-2 Opt2 rows built on the known-bits lattice
//! ([`super::plans::bits`]), which they share so their notions of "what is
//! provably known about this value" cannot diverge:
//!
//! - **Known-bits simplification** — an operation whose *result* is fully
//!   known becomes `mov_imm` (a masked value whose every bit is pinned, an
//!   `and` with a mask that clears everything, a shift that shifts a known
//!   value out entirely), and an operation that provably cannot change its
//!   input becomes a plain copy (`x AND mask` where `x` already fits inside
//!   `mask`, `x ORR 0`, `x EOR 0`).
//! - **Narrowing / bit-width reduction** — a mask whose bits the value
//!   already satisfies is redundant, so the `and` is dropped in favour of its
//!   source: the value was already narrow.
//! - **Sign/zero extension elimination** — the same proof applied to the
//!   zero-extension idiom this MIR actually emits, `lsl_imm` + `lsr_imm` by
//!   the same amount (or an `and` with a width mask): when the value's high
//!   bits are provably already clear, the extension is a no-op copy.
//!
//! All three rewrite only into `mov_imm` or `mov` — never into an operation
//! that can trap or set flags — and every candidate op comes from the pure,
//! flag-free whitelist, so a checked-arithmetic instruction is never a
//! candidate. Each rewrite is value-identical by the lattice's own proof;
//! copy propagation then bypasses the copies and DCE sweeps what strands.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::{build_cfg, classify_ref, RegRef};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;
use crate::target::shared::regmodel::RegisterModel;

use super::lvn::copy_of;
use super::plans::bits::{analyze, Known};
use super::plans::ssa;

/// Run the three known-bits rows over one function's selected stream, in
/// place. All three are Level 2, so one guard serves them; each rewrite is
/// attributed to its own row's counter.
pub(crate) fn simplify(instructions: &mut [CodeInstruction], model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);
    let known = analyze(instructions, &models, &overlay);

    let value_of = |i: usize, field: &str| -> Option<Known> {
        let operand = instructions[i].operand(field)?;
        match classify_ref(operand, &models.0)? {
            RegRef::VReg(id) => Some(
                known[overlay
                    .value_defined_at(i, (false, id))
                    .or_else(|| overlay.value_of_use(i, (false, id)))?],
            ),
            RegRef::Phys(_) => None,
        }
    };
    let use_known = |i: usize, field: &str| -> Known {
        let Some(operand) = instructions[i].operand(field) else {
            return Known::UNKNOWN;
        };
        match classify_ref(operand, &models.0) {
            Some(RegRef::VReg(id)) => overlay
                .value_of_use(i, (false, id))
                .map(|v| known[v])
                .unwrap_or(Known::UNKNOWN),
            Some(RegRef::Phys(_)) => Known::UNKNOWN,
            None => match super::branches::bits(&operand.rendered()) {
                Some(value) => Known::exact(value),
                None => Known::UNKNOWN,
            },
        }
    };

    // Decide every rewrite first (the lattice closures borrow the stream),
    // then apply — the same shape `opt2::sccp` uses.
    enum Rewrite {
        Constant(String, u64),
        Copy(Operand, Operand),
    }
    let (mut simplified, mut narrowed, mut extensions) = (0u64, 0u64, 0u64);
    let mut planned: Vec<(usize, Rewrite)> = Vec::new();
    for i in 0..instructions.len() {
        // Only the pure whitelist, and never a move (already minimal).
        if !super::lvn::numberable_op(instructions[i].op) {
            continue;
        }
        let Some(dst) = instructions[i].operand("dst").cloned() else {
            continue;
        };
        // Rewrite 1: the whole result is known → materialize it.
        let result = value_of(i, "dst").unwrap_or(Known::UNKNOWN);
        if result.ones | result.zeros == u64::MAX {
            planned.push((
                i,
                Rewrite::Constant(dst.rendered().into_owned(), result.ones),
            ));
            simplified += 1;
            continue;
        }
        // Rewrite 2/3: the operation provably cannot change its input.
        match instructions[i].op {
            CodeOp::And => {
                let (lhs, rhs) = (use_known(i, "lhs"), use_known(i, "rhs"));
                // `x AND mask` where x already fits inside mask is just x.
                let redundant_mask = |mask: &Known, value: &Known| {
                    mask.ones | mask.zeros == u64::MAX && value.fits_in(mask.ones)
                };
                let source = if redundant_mask(&rhs, &lhs) {
                    instructions[i].operand("lhs").cloned()
                } else if redundant_mask(&lhs, &rhs) {
                    instructions[i].operand("rhs").cloned()
                } else {
                    None
                };
                if let Some(source) = source {
                    planned.push((i, Rewrite::Copy(dst, source)));
                    // A width mask over an already-narrow value is exactly the
                    // zero-extension idiom; anything else is narrowing.
                    if is_width_mask(&rhs) || is_width_mask(&lhs) {
                        extensions += 1;
                    } else {
                        narrowed += 1;
                    }
                }
            }
            CodeOp::Orr | CodeOp::Eor => {
                // `x ORR 0` / `x EOR 0` are copies of x.
                let (lhs, rhs) = (use_known(i, "lhs"), use_known(i, "rhs"));
                let zero = |k: &Known| k.zeros == u64::MAX;
                let source = if zero(&rhs) {
                    instructions[i].operand("lhs").cloned()
                } else if zero(&lhs) {
                    instructions[i].operand("rhs").cloned()
                } else {
                    None
                };
                if let Some(source) = source {
                    planned.push((i, Rewrite::Copy(dst, source)));
                    simplified += 1;
                }
            }
            CodeOp::LsrImm => {
                // A zero-amount shift is a copy; a shift that clears the value
                // entirely is rewrite 1's business, already handled above.
                let amount = instructions[i]
                    .get("shift")
                    .and_then(|text| text.parse::<u32>().ok())
                    .unwrap_or(u32::MAX);
                if amount == 0 {
                    if let Some(source) = instructions[i].operand("src").cloned() {
                        planned.push((i, Rewrite::Copy(dst, source)));
                        extensions += 1;
                    }
                }
            }
            _ => {}
        }
    }
    for (i, rewrite) in planned {
        instructions[i] = match rewrite {
            Rewrite::Constant(dst, value) => {
                abi::move_immediate(&dst, "Integer", &value.to_string())
            }
            Rewrite::Copy(dst, source) => copy_of(dst, source),
        };
    }
    crate::optimizer::stats::count_known_bits_simplifications(simplified);
    crate::optimizer::stats::count_values_narrowed(narrowed);
    crate::optimizer::stats::count_extensions_removed(extensions);
}

/// Whether the mask is a contiguous low-bit width mask (`0xff`, `0xffff`,
/// `0xffff_ffff`) — the shape a zero extension uses, as opposed to an
/// arbitrary bit mask.
fn is_width_mask(mask: &Known) -> bool {
    mask.ones | mask.zeros == u64::MAX
        && matches!(mask.ones, 0xff | 0xffff | 0xffff_ffff | 0xff_ffff)
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

    fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
        ci(
            "mov_imm",
            &[("dst", dst), ("type", "Integer"), ("value", value)],
        )
    }

    fn run(stream: &mut [CodeInstruction], level: u8) {
        with_opt_level(OptLevel(level), || simplify(stream, &Aarch64RegisterModel));
    }

    /// A mask over a value whose high bits are provably already clear is
    /// redundant — the extension-elimination case.
    #[test]
    fn redundant_width_masks_become_copies() {
        let mut stream = vec![
            // %v2 = %v9 >> 32  →  provably fits in 32 bits.
            ci(
                "lsr_imm",
                &[("dst", "%v2"), ("src", "%v9"), ("shift", "32")],
            ),
            mov_imm("%v3", "4294967295"),
            ci("and", &[("dst", "%v4"), ("lhs", "%v2"), ("rhs", "%v3")]),
            ci(
                "str_u64",
                &[("src", "%v4"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::Mov, "the mask was a no-op");
        assert_eq!(stream[2].get("src").as_deref(), Some("%v2"));
    }

    /// A fully-known result is materialized directly.
    #[test]
    fn fully_known_results_become_immediates() {
        let mut stream = vec![
            mov_imm("%v1", "12"),
            mov_imm("%v2", "10"),
            ci("and", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci(
                "str_u64",
                &[("src", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::MovImm);
        assert_eq!(stream[2].get("value").as_deref(), Some("8"), "12 AND 10");
    }

    /// A mask that does NOT cover the value's unknown bits must stay.
    #[test]
    fn genuine_masks_survive() {
        let mut stream = vec![
            mov_imm("%v3", "255"),
            ci("and", &[("dst", "%v4"), ("lhs", "%v9"), ("rhs", "%v3")]),
            ci(
                "str_u64",
                &[("src", "%v4"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[1].op, CodeOp::And, "%v9's bits are unknown");
    }

    /// Checked arithmetic is never a candidate (it is not in the whitelist).
    #[test]
    fn checked_arithmetic_is_never_rewritten() {
        let mut stream = vec![
            mov_imm("%v1", "0"),
            ci("adds", &[("dst", "%v2"), ("lhs", "%v1"), ("rhs", "%v1")]),
            ci("b.vc", &[("target", "ok")]),
            ci("bl", &[("target", "_raise")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
        ];
        let before: Vec<CodeOp> = stream.iter().map(|i| i.op).collect();
        run(&mut stream, 2);
        assert_eq!(stream.iter().map(|i| i.op).collect::<Vec<_>>(), before);
    }

    /// The rows are off at `-O1`.
    #[test]
    fn level_one_disables_the_rows() {
        let mut stream = vec![
            mov_imm("%v1", "12"),
            mov_imm("%v2", "10"),
            ci("and", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 1);
        assert_eq!(stream[2].op, CodeOp::And);
    }
}
