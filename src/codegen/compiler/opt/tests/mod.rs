//! `fuse_scalar_fma` over the malformed and unusual streams it must leave alone.
//!
//! The recognizer is a peephole over a function's float-op stream and it REWRITES, so
//! each guard in it is a miscompile if it is wrong. `fma_fusion.rs`'s own tests cover
//! the fusions and the headline refusals (a multi-use product, a physical-register
//! kernel, a redefinition in the span). What they do not cover is a stream that is not
//! shaped like the emitter's: an instruction missing a field, a product whose single
//! use is not after the multiply, a branch target between the two ops, an addend that
//! is not a register. In every one of those the pass must change NOTHING — a fusion
//! it was unsure of is a double rounding it cannot take back.

use crate::arch::ops::CodeOp;
use crate::codegen::compiler::opt::fma_fusion::fuse_scalar_fma;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.physical_field(k, Operand::parse(v));
    }
    inst
}

fn mul() -> CodeInstruction {
    ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")])
}

/// The ops of `stream`, for asserting that nothing was rewritten or removed.
fn ops(stream: &[CodeInstruction]) -> Vec<CodeOp> {
    stream.iter().map(|inst| inst.op).collect()
}

fn assert_untouched(mut stream: Vec<CodeInstruction>, why: &str) {
    let before = ops(&stream);
    fuse_scalar_fma(&mut stream);
    assert_eq!(
        ops(&stream),
        before,
        "{why}: the stream must be left exactly as it was"
    );
}

/// A multiply missing an operand field is not a multiply this pass understands.
#[test]
fn a_multiply_missing_a_field_is_left_alone() {
    assert_untouched(
        vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ],
        "a multiply with no rhs",
    );
}

/// A product whose one use comes BEFORE the multiply has no consumer after it.
#[test]
fn a_product_whose_only_use_precedes_the_multiply_has_no_consumer() {
    assert_untouched(
        vec![
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
            mul(),
        ],
        "the only reader of %f2 is above its definition",
    );
}

/// A branch target between the multiply and the add means control can reach the add
/// without having run the multiply.
#[test]
fn a_label_between_the_multiply_and_the_add_blocks_fusion() {
    assert_untouched(
        vec![
            mul(),
            CodeInstruction::new("label").physical_field("name", Operand::parse("join")),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ],
        "a label sits between the two ops",
    );
}

/// The consumer must name a destination, and the addend must be a register, in each of
/// the four fusable shapes (add either way round, subtract either way round).
#[test]
fn a_consumer_with_no_destination_or_no_addend_is_left_alone() {
    // No `dst` on the consumer.
    assert_untouched(
        vec![mul(), ci("fadd_d", &[("lhs", "%f2"), ("rhs", "%f9")])],
        "the add names no destination",
    );
    // No addend operand in each shape: the product sits in one operand and the other
    // is absent.
    for (op, product_side) in [
        ("fadd_d", "lhs"),
        ("fadd_d", "rhs"),
        ("fsub_d", "lhs"),
        ("fsub_d", "rhs"),
    ] {
        assert_untouched(
            vec![mul(), ci(op, &[("dst", "%f3"), (product_side, "%f2")])],
            &format!("{op} with the product on the {product_side} and no addend"),
        );
    }
}

/// A product that appears only in a non-operand field of the consumer is not fusable.
#[test]
fn a_product_in_a_non_operand_field_is_left_alone() {
    assert_untouched(
        vec![
            mul(),
            ci(
                "fadd_d",
                &[
                    ("dst", "%f3"),
                    ("lhs", "%f8"),
                    ("rhs", "%f9"),
                    ("addend", "%f2"),
                ],
            ),
        ],
        "the product is named by a field that is neither operand",
    );
}
