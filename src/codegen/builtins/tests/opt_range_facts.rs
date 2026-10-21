//! The integer range lattice (`optimizer::opt2::plans::ranges`) at transfer
//! and comparison cases the check-elision tests do not reach, observed through
//! the comparison guard it decides (`optimizer::opt2::checks`): `%v2 < 10`
//! after `%v2` is computed from a known `5`.
//!
//! Exactly known: a zero-amount logical right shift of a known value.
//! Unknown (so the guard stays): an immediate that does not parse, a shift
//! with no readable amount (logical right, arithmetic right, left), an operand
//! that is missing, and a comparison whose other side is missing or is a
//! literal that does not parse. The lattice is also total on an empty stream.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::build_cfg;
use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::{with_opt_level, OptLevel};

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut instruction = CodeInstruction::new(op);
    for (name, value) in fields {
        instruction = instruction.physical_field(name, *value);
    }
    instruction
}

fn mov_imm(dst: &'static str, value: &'static str) -> CodeInstruction {
    ci(
        "mov_imm",
        &[("dst", dst), ("type", "Integer"), ("value", value)],
    )
}

/// `compute` produces `%v2`; then `cmp %v2, 10; b.lt ok` with a raise on the
/// fall-through. Whether the guard was decided (made unconditional).
fn decided(compute: Vec<CodeInstruction>, compare: CodeInstruction) -> bool {
    let mut stream = compute;
    let guard = stream.len() + 1;
    stream.extend([
        compare,
        ci("b.lt", &[("target", "ok")]),
        ci("bl", &[("target", "_mfb_make_error_result")]),
        ci("label", &[("name", "ok")]),
        ci("ret", &[]),
    ]);
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(3), || {
        crate::optimizer::opt2::checks::eliminate(&mut stream, &model)
    });
    stream[guard].op == CodeOp::Branch
}

fn less_than_ten() -> CodeInstruction {
    ci("cmp_imm", &[("lhs", "%v2"), ("rhs", "10")])
}

#[test]
fn a_zero_amount_right_shift_of_a_known_value_decides_the_guard() {
    assert!(decided(
        vec![
            mov_imm("%v1", "5"),
            ci("lsr_imm", &[("dst", "%v2"), ("src", "%v1"), ("shift", "0")]),
        ],
        less_than_ten(),
    ));
}

#[test]
fn unreadable_transfers_leave_the_guard() {
    let cases: Vec<(&str, Vec<CodeInstruction>)> = vec![
        ("an unparseable immediate", vec![mov_imm("%v2", "five")]),
        (
            "a logical right shift with no amount",
            vec![
                mov_imm("%v1", "5"),
                ci("lsr_imm", &[("dst", "%v2"), ("src", "%v1")]),
            ],
        ),
        (
            "an arithmetic right shift with no amount",
            vec![
                mov_imm("%v1", "5"),
                ci("asr_imm", &[("dst", "%v2"), ("src", "%v1")]),
            ],
        ),
        (
            "a left shift with no amount",
            vec![
                mov_imm("%v1", "5"),
                ci("lsl_imm", &[("dst", "%v2"), ("src", "%v1")]),
            ],
        ),
        (
            "an add with a missing operand",
            vec![
                mov_imm("%v1", "5"),
                ci("add", &[("dst", "%v2"), ("lhs", "%v1")]),
            ],
        ),
    ];
    for (what, compute) in cases {
        assert!(!decided(compute, less_than_ten()), "{what}");
    }
}

#[test]
fn a_comparison_with_an_unreadable_side_decides_nothing() {
    assert!(!decided(
        vec![mov_imm("%v2", "5")],
        ci("cmp_imm", &[("lhs", "%v2")]),
    ));
    assert!(!decided(
        vec![mov_imm("%v2", "5")],
        ci("cmp_imm", &[("lhs", "%v2"), ("rhs", "ten")]),
    ));
}

#[test]
fn the_lattice_is_total_on_an_empty_stream() {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    let models = regalloc::class_models(&model);
    let stream: Vec<CodeInstruction> = Vec::new();
    let blocks = build_cfg(&stream);
    let overlay = crate::optimizer::opt2::plans::ssa::build(&stream, &blocks, &models);
    let _ = crate::optimizer::opt2::plans::ranges::analyze(&stream, &models, &overlay, &blocks);
    assert!(crate::optimizer::opt2::plans::ranges::reverse_postorder(&blocks).is_empty());
}
