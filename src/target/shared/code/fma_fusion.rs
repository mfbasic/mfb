//! Scalar fused-multiply-add recognizer (plan-02 §5).
//!
//! A peephole over a function's lowered MIR float-op stream that rewrites a
//! **single-use** `fmul_d` feeding an `fadd_d`/`fsub_d` into one single-rounded
//! fused op:
//!
//! ```text
//!   fmul_d  %p, a, b            fmadd_d  %w, c, a, b   ; w = a*b + c
//!   fadd_d  %w, %p, c     =>    (fmul removed)
//! ```
//!
//! The sign combinations map to the neutral fused ops (each backend selects the
//! native form that computes the same result — see the `CodeOp` docs). Both add
//! orderings (`a*b + c` and the commuted `c + a*b`) fuse to `fmadd_d`:
//!
//! | source            | fused op    | result            |
//! |-------------------|-------------|-------------------|
//! | `a*b + c`         | `fmadd_d`   | `c + a*b`         |
//! | `c + a*b`         | `fmadd_d`   | `c + a*b`         |
//! | `a*b - c`         | `fmsub_d`   | `a*b - c`         |
//! | `c - a*b`         | `fnmsub_d`  | `c - a*b`         |
//!
//! Fusion is applied only when
//!  * the multiply's result `%p` is a floating-point virtual register used
//!    **exactly once** (so the un-rounded product is never separately observed),
//!  * both multiply operands are FP virtual registers (this restricts fusion to
//!    the `d`-native user-expression path; the physical-`d` transcendental kernels
//!    are left untouched — they are already hand-fused), and
//!  * neither multiply operand is redefined between the `fmul_d` and its consumer.
//!
//! Runs before register allocation, on virtual registers. It changes results
//! (single vs. double rounding) and so is gated by the ULP harness and wired into
//! `lower_function` only in plan-02 Phase 3.

use super::regalloc::parse_fp_vreg;
use super::types::CodeInstruction;
use crate::arch::ops::CodeOp;
use crate::target::shared::abi;

/// Field names that *write* a register (everything else that names a register is a
/// read). Mirrors `regalloc::analysis::DEF_FIELDS`.
fn is_def_field(name: &str) -> bool {
    matches!(name, "dst" | "carry_out" | "borrow_out")
}

/// Count, per register token, how many times it appears in a *use* (read) field
/// across the whole instruction list.
fn use_counts(instructions: &[CodeInstruction]) -> std::collections::HashMap<String, u32> {
    let mut counts = std::collections::HashMap::new();
    for inst in instructions {
        for (name, value) in &inst.fields {
            if !is_def_field(name) {
                *counts.entry(value.clone()).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Rewrite single-use `a*b (+|-) c` chains into fused multiply-add ops, in place.
/// Called by `lower_function` just before register allocation (plan-02 Phase 3).
pub(crate) fn fuse_scalar_fma(instructions: &mut Vec<CodeInstruction>) {
    let counts = use_counts(instructions);
    let mut remove = vec![false; instructions.len()];

    for i in 0..instructions.len() {
        if instructions[i].op != CodeOp::FMulD {
            continue;
        }
        let (product, a, b) = match (
            instructions[i].get("dst"),
            instructions[i].get("lhs"),
            instructions[i].get("rhs"),
        ) {
            (Some(d), Some(l), Some(r)) => (d.to_string(), l.to_string(), r.to_string()),
            _ => continue,
        };
        // Restrict to the d-native user path: product and both operands are FP
        // vregs, product is used exactly once, and it is not one of its own inputs.
        if parse_fp_vreg(&product).is_none()
            || parse_fp_vreg(&a).is_none()
            || parse_fp_vreg(&b).is_none()
            || product == a
            || product == b
            || counts.get(&product).copied().unwrap_or(0) != 1
        {
            continue;
        }

        // Find the single consumer (the one instruction that reads `product`).
        let Some(j) = (i + 1..instructions.len()).find(|&k| {
            instructions[k]
                .fields
                .iter()
                .any(|(name, v)| !is_def_field(name) && v == &product)
        }) else {
            continue;
        };

        // The consumer must be an add/sub reading `product` in a register operand,
        // and neither multiply operand — nor the product itself — may be redefined
        // in between (else the fused form would read a stale value, or fold across
        // a redefinition that overwrote the un-rounded product).
        let consumer_op = instructions[j].op;
        if consumer_op != CodeOp::FAddD && consumer_op != CodeOp::FSubD {
            continue;
        }
        // `product` is used exactly once (checked above), but a *definition* is not
        // a use, so `use_counts` cannot see a redefinition of `%p` between the
        // multiply and its consumer. Guard it explicitly: a fresh def of `%p` in the
        // span means the consumer reads that new value, not `a*b`. Today the product
        // is always a single-def fresh vreg (`emit_float_binary`), so this never
        // fires and the emitted code is unchanged; it converts a future reused
        // product vreg from a silent miscompile into a skipped fusion.
        let redefined_between = instructions[i + 1..j].iter().any(|inst| {
            inst.fields
                .iter()
                .any(|(name, v)| is_def_field(name) && (v == &a || v == &b || v == &product))
        });
        if redefined_between {
            continue;
        }
        // A branch target (`Label`) anywhere between the multiply and its consumer
        // means control can reach the `fadd`/`fsub` without having executed the
        // `fmul` — the fused op would then read an operand that a different path
        // redefined, or the un-rounded product that was never computed on that
        // edge. Linear single-use/redefinition reasoning does not cover that, so
        // bail. Today the d-native emitter never lays a label between the two ops,
        // so this never fires and the emitted code is unchanged; it converts a
        // future cross-label chain from a silent miscompile into a skipped fusion.
        if instructions[i + 1..=j]
            .iter()
            .any(|inst| inst.op == CodeOp::Label)
        {
            continue;
        }

        let dst = match instructions[j].get("dst") {
            Some(d) => d.to_string(),
            None => continue,
        };
        let lhs = instructions[j].get("lhs").unwrap_or_default().to_string();
        let rhs = instructions[j].get("rhs").unwrap_or_default().to_string();

        // Determine the fused op from which operand holds the product and whether
        // the consumer adds or subtracts. `c` is the other (addend) operand.
        let fused = if consumer_op == CodeOp::FAddD && lhs == product {
            // w = product + rhs  →  rhs + a*b
            abi::float_multiply_add_d(&dst, &rhs, &a, &b)
        } else if consumer_op == CodeOp::FAddD && rhs == product {
            // w = lhs + product  →  lhs + a*b
            abi::float_multiply_add_d(&dst, &lhs, &a, &b)
        } else if consumer_op == CodeOp::FSubD && lhs == product {
            // w = product - rhs  →  a*b - rhs
            abi::float_multiply_sub_d(&dst, &rhs, &a, &b)
        } else if consumer_op == CodeOp::FSubD && rhs == product {
            // w = lhs - product  →  lhs - a*b
            abi::float_negate_multiply_sub_d(&dst, &lhs, &a, &b)
        } else {
            // `product` appears in a non-register field (should not happen for
            // fadd/fsub) — leave untouched.
            continue;
        };

        instructions[j] = fused;
        remove[i] = true;
    }

    if remove.iter().any(|&r| r) {
        let mut idx = 0;
        instructions.retain(|_| {
            let keep = !remove[idx];
            idx += 1;
            keep
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    /// `%2 = %0 * %1 ; %3 = %2 + %c` fuses to `fmadd_d %3, %c, %0, %1`.
    #[test]
    fn fuses_multiply_then_add() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 1);
        assert_eq!(ins[0].op, CodeOp::FMaddD);
        assert_eq!(ins[0].get("dst"), Some("%f3"));
        assert_eq!(ins[0].get("addend"), Some("%f9"));
        assert_eq!(ins[0].get("lhs"), Some("%f0"));
        assert_eq!(ins[0].get("rhs"), Some("%f1"));
    }

    /// The product in the right operand still fuses to `fmadd_d` (commuted add).
    #[test]
    fn fuses_add_with_product_on_right() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f9"), ("rhs", "%f2")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 1);
        assert_eq!(ins[0].op, CodeOp::FMaddD);
        assert_eq!(ins[0].get("addend"), Some("%f9"));
    }

    /// `a*b - c` → `fmsub_d`; `c - a*b` → `fnmsub_d`.
    #[test]
    fn fuses_subtract_both_directions() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fsub_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins[0].op, CodeOp::FMsubD);
        assert_eq!(ins[0].get("addend"), Some("%f9"));

        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fsub_d", &[("dst", "%f3"), ("lhs", "%f9"), ("rhs", "%f2")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins[0].op, CodeOp::FNmsubD);
        assert_eq!(ins[0].get("addend"), Some("%f9"));
    }

    /// A product used twice (also stored) is NOT fused — the un-rounded value is
    /// observed elsewhere.
    #[test]
    fn does_not_fuse_multiuse_product() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
            ci("str_d", &[("src", "%f2"), ("base", "sp"), ("offset", "0")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 3);
        assert_eq!(ins[0].op, CodeOp::FMulD);
    }

    /// A non-add/sub consumer (another multiply) is not fused.
    #[test]
    fn does_not_fuse_non_addsub_consumer() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fmul_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 2);
        assert_eq!(ins[0].op, CodeOp::FMulD);
    }

    /// Physical `d`-register kernels (not FP vregs) are left untouched.
    #[test]
    fn does_not_fuse_physical_registers() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "d0"), ("lhs", "d0"), ("rhs", "d2")]),
            ci("fadd_d", &[("dst", "d0"), ("lhs", "d0"), ("rhs", "d1")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 2);
        assert_eq!(ins[0].op, CodeOp::FMulD);
    }

    /// If a multiply operand is redefined before the consumer, do not fuse.
    #[test]
    fn does_not_fuse_when_operand_redefined() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fmov_d_from_d", &[("dst", "%f0"), ("src", "%f5")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 3);
        assert_eq!(ins[0].op, CodeOp::FMulD);
    }

    /// If the *product* vreg is redefined between the multiply and the add, the add
    /// no longer reads `a*b` and the chain must NOT be fused. (`%f2` is still read
    /// exactly once — by the `fadd_d` — so `use_counts` alone cannot catch this; the
    /// product-redefinition guard must.)
    #[test]
    fn does_not_fuse_when_product_redefined() {
        let mut ins = vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            // `%f2` re-defined here to an unrelated value.
            ci("fmov_d_from_d", &[("dst", "%f2"), ("src", "%f5")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ];
        fuse_scalar_fma(&mut ins);
        assert_eq!(ins.len(), 3);
        assert_eq!(ins[0].op, CodeOp::FMulD);
        assert_eq!(ins[2].op, CodeOp::FAddD);
    }
}
