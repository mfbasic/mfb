//! The known-bits lattice (`optimizer::opt2::plans::bits`) at the transfer
//! cases its consumers' tests do not reach, observed through the known-bits
//! row that rewrites a fully-known result into `mov_imm`:
//!
//! - an `add` of two fully-known values keeps every bit below the top one (the
//!   carry out of the known run is unknown territory), so masking its low bits
//!   is fully known;
//! - a `mov_imm` whose value does not parse, a shift with no readable amount, a
//!   shift by 64 or more (ISA-divergent, never modeled) are UNKNOWN;
//! - a zero-amount right shift of a known value keeps it exactly.

use crate::arch::ops::CodeOp;
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

fn store(src: &'static str) -> CodeInstruction {
    ci("str_u64", &[("src", src), ("base", "sp"), ("offset", "8")])
}

fn run(stream: &mut [CodeInstruction]) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(2), || {
        crate::optimizer::opt2::knownbits::simplify(stream, &model)
    });
}

#[test]
fn an_add_of_two_known_values_is_known_below_its_carry() {
    let mut stream = vec![
        mov_imm("%v1", "5"),
        mov_imm("%v2", "6"),
        ci("orr", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("add", &[("dst", "%v4"), ("lhs", "%v3"), ("rhs", "%v1")]),
        ci("and", &[("dst", "%v5"), ("lhs", "%v4"), ("rhs", "15")]),
        store("%v5"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    // `5 | 6` is 7 (rewritten too); `7 + 5` is 12 in every bit but the top
    // (the carry out of the known run is unknown territory), so
    // `(7 + 5) & 15` is exactly 12.
    assert_eq!(stream[2].op, CodeOp::MovImm);
    assert_eq!(stream[2].get("value").as_deref(), Some("7"));
    assert_eq!(stream[3].op, CodeOp::Add, "the add's top bit is not known");
    assert_eq!(stream[4].op, CodeOp::MovImm, "its low nibble is");
    assert_eq!(stream[4].get("value").as_deref(), Some("12"));
}

#[test]
fn a_zero_amount_right_shift_of_a_known_value_keeps_it() {
    let mut stream = vec![
        mov_imm("%v1", "40"),
        ci("lsr_imm", &[("dst", "%v2"), ("src", "%v1"), ("shift", "0")]),
        ci("orr", &[("dst", "%v3"), ("lhs", "%v2"), ("rhs", "%v1")]),
        store("%v3"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(stream[2].op, CodeOp::MovImm);
    assert_eq!(stream[2].get("value").as_deref(), Some("40"));
}

#[test]
fn unreadable_immediates_and_unmodelled_shifts_are_unknown() {
    let shapes = || {
        vec![
            mov_imm("%v1", "not-a-number"),
            mov_imm("%v2", "12"),
            // No readable amount, and an amount the ISAs disagree on.
            ci("lsl_imm", &[("dst", "%v3"), ("src", "%v2")]),
            ci(
                "lsr_imm",
                &[("dst", "%v4"), ("src", "%v2"), ("shift", "64")],
            ),
            ci("orr", &[("dst", "%v5"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("orr", &[("dst", "%v6"), ("lhs", "%v3"), ("rhs", "%v2")]),
            ci("orr", &[("dst", "%v7"), ("lhs", "%v4"), ("rhs", "%v2")]),
            store("%v5"),
            store("%v6"),
            store("%v7"),
            ci("ret", &[]),
        ]
    };
    let mut stream = shapes();
    run(&mut stream);
    for (index, instruction) in stream.iter().enumerate().take(7).skip(4) {
        assert_eq!(
            instruction.op,
            CodeOp::Orr,
            "an orr with an UNKNOWN operand is not materialized (index {index})"
        );
    }
}
