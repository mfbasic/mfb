//! The overflow-guard half of the range-driven check elision
//! (`optimizer::opt2::checks`): a `b.vc` guard becomes unconditional only when
//! the lattice proves the governing `adds`/`subs` cannot set `V`, and only that
//! flag-setter may govern it — nothing but provably flag-free instructions may
//! sit between them.
//!
//! The row's own tests use register operands; these add a literal operand
//! (exactly known), a physical or missing operand (unbounded), a flag-free
//! instruction between the add and its guard (looked through), another flag
//! toucher there (no proof), a guard with no flag-setter in its block at all,
//! and an empty stream.

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

fn run(stream: &mut Vec<CodeInstruction>) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(3), || {
        crate::optimizer::opt2::checks::eliminate(stream, &model)
    });
}

/// `prefix` then `b.vc ok`, a raise on the fall-through, and `ok:` — returns
/// whether the guard was made unconditional.
fn guard_removed(prefix: Vec<CodeInstruction>) -> bool {
    let mut stream = prefix;
    let guard = stream.len();
    stream.extend([
        ci("b.vc", &[("target", "overflow_ok_1")]),
        ci("bl", &[("target", "_mfb_make_error_result")]),
        ci("label", &[("name", "overflow_ok_1")]),
        ci("ret", &[]),
    ]);
    run(&mut stream);
    match stream[guard].op {
        CodeOp::Branch => true,
        CodeOp::BranchVc => false,
        other => panic!("the guard became {other:?}"),
    }
}

#[test]
fn an_empty_stream_is_left_empty() {
    let mut stream = Vec::new();
    run(&mut stream);
    assert!(stream.is_empty());
}

#[test]
fn a_literal_operand_is_exactly_known() {
    assert!(guard_removed(vec![
        mov_imm("%v1", "2"),
        ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "5")]),
    ]));
}

#[test]
fn a_flag_free_instruction_between_the_add_and_its_guard_is_looked_through() {
    assert!(guard_removed(vec![
        mov_imm("%v1", "2"),
        ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "5")]),
        ci("mov", &[("dst", "%v4"), ("src", "%v3")]),
    ]));
}

#[test]
fn an_unbounded_operand_keeps_the_guard() {
    assert!(
        !guard_removed(vec![ci(
            "adds",
            &[("dst", "%v3"), ("lhs", "x3"), ("rhs", "5")]
        )]),
        "a physical register's value is not modeled"
    );
    assert!(
        !guard_removed(vec![
            mov_imm("%v1", "2"),
            ci("adds", &[("dst", "%v3"), ("lhs", "%v1")])
        ]),
        "a missing operand is unbounded"
    );
}

#[test]
fn a_guard_not_governed_by_a_provable_add_keeps_it() {
    assert!(
        !guard_removed(vec![
            mov_imm("%v1", "2"),
            ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "5")]),
            ci("cmp_imm", &[("lhs", "%v3"), ("rhs", "0")]),
        ]),
        "another flag-setter sits between the add and the guard"
    );
    assert!(
        !guard_removed(vec![mov_imm("%v1", "2")]),
        "no flag-setter in the block at all"
    );
}
