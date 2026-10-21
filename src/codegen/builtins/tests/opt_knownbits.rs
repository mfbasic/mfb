//! The known-bits rows (`optimizer::opt2::knownbits`) at the operand shapes
//! their inline tests do not use: a literal mask (on either side), an
//! arbitrary non-width mask (narrowing rather than extension elimination), a
//! zero-amount shift, a physical-register or missing operand, a whitelisted op
//! with no destination, and an empty stream.
//!
//! The contract: a rewrite happens only where the lattice proves the result
//! value-identical — the op becomes a plain `mov` of the operand it cannot
//! change — and an operand the lattice cannot see (a physical register, a
//! missing field) is UNKNOWN, never assumed.

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

fn run(stream: &mut [CodeInstruction]) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(2), || {
        crate::optimizer::opt2::knownbits::simplify(stream, &model)
    });
}

fn store(src: &'static str) -> CodeInstruction {
    ci("str_u64", &[("src", src), ("base", "sp"), ("offset", "8")])
}

#[test]
fn an_empty_stream_is_left_empty() {
    let mut stream = Vec::new();
    run(&mut stream);
    assert!(stream.is_empty());
}

/// `%v1` holds at most the low nibble; masking it again with a literal width
/// mask (on either side) or with a wider arbitrary mask cannot change it.
#[test]
fn literal_masks_over_an_already_narrow_value_become_copies() {
    let mut stream = vec![
        ci("and", &[("dst", "%v1"), ("lhs", "%v0"), ("rhs", "15")]),
        // A width mask on the right: the extension idiom.
        ci("and", &[("dst", "%v2"), ("lhs", "%v1"), ("rhs", "255")]),
        // The same mask written on the left.
        ci("and", &[("dst", "%v3"), ("lhs", "255"), ("rhs", "%v1")]),
        // 0x0f0f is not a width mask: this is narrowing.
        ci("and", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "3855")]),
        store("%v2"),
        store("%v3"),
        store("%v4"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(stream[0].op, CodeOp::And, "%v0's low nibble is unknown");
    for (index, dst) in [(1, "%v2"), (2, "%v3"), (3, "%v4")] {
        assert_eq!(stream[index].op, CodeOp::Mov, "{dst}'s mask is a no-op");
        assert_eq!(stream[index].get("dst").as_deref(), Some(dst));
        assert_eq!(stream[index].get("src").as_deref(), Some("%v1"));
    }
}

#[test]
fn a_zero_amount_shift_becomes_a_copy() {
    let mut stream = vec![
        ci("lsr_imm", &[("dst", "%v2"), ("src", "%v0"), ("shift", "0")]),
        ci("lsr_imm", &[("dst", "%v3"), ("src", "%v0"), ("shift", "3")]),
        store("%v2"),
        store("%v3"),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(stream[0].op, CodeOp::Mov);
    assert_eq!(stream[0].get("src").as_deref(), Some("%v0"));
    assert_eq!(stream[1].op, CodeOp::LsrImm, "a real shift stays");
}

/// What the lattice cannot see stays as written: a physical-register
/// operand, a missing operand, and a whitelisted op with no destination.
#[test]
fn operands_the_lattice_cannot_see_block_every_rewrite() {
    let shapes = || {
        vec![
            ci("and", &[("dst", "%v1"), ("lhs", "%v0"), ("rhs", "15")]),
            ci("and", &[("dst", "%v2"), ("lhs", "x3"), ("rhs", "%v1")]),
            ci("orr", &[("dst", "%v5"), ("lhs", "%v1")]),
            ci("add", &[("lhs", "%v1"), ("rhs", "%v1")]),
            store("%v2"),
            store("%v5"),
            ci("ret", &[]),
        ]
    };
    let mut stream = shapes();
    run(&mut stream);
    let ops: Vec<CodeOp> = stream.iter().map(|i| i.op).collect();
    let before: Vec<CodeOp> = shapes().iter().map(|i| i.op).collect();
    assert_eq!(ops, before);
}
