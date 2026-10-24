//! Partial redundancy elimination (`optimizer::opt2::pre`) over the CFG shapes
//! its own inline tests do not build: a back edge, two diamonds in one stream,
//! a gap block that opens with a label or closes with a flag setter, an operand
//! that is a literal or a loop-carried φ, and the two declines a loop exposes.
//!
//! The contract is the row's own: an expression available on every predecessor
//! but one is computed on that one (only where the predecessor always reaches
//! the join and the operands are already in hand), the join's copy goes, and the
//! stream keeps its length — PRE never grows the program.

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

fn ops(stream: &[CodeInstruction]) -> Vec<CodeOp> {
    stream.iter().map(|instruction| instruction.op).collect()
}

fn run(stream: &mut Vec<CodeInstruction>) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(3), || {
        crate::optimizer::opt2::pre::eliminate(stream, &model)
    });
}

/// The `src` of the first `mov` writing `dst`.
fn mov_source(stream: &[CodeInstruction], dst: &str) -> Option<String> {
    stream
        .iter()
        .find(|i| i.op == CodeOp::Mov && i.get("dst").as_deref() == Some(dst))
        .and_then(|i| i.get("src"))
}

/// The loop's preheader lacks `a * b`, the latch computes it, and the header
/// recomputes it. The preheader always reaches the header, so it takes the
/// computation and the header's copy goes; the header's use reads the latch's
/// register on both entries.
#[test]
fn a_loop_header_recomputation_moves_into_the_preheader() {
    let mut stream = vec![
        ci("b", &[("target", "head")]),
        ci("label", &[("name", "head")]),
        ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "latch")]),
        ci("ret", &[]),
        ci("label", &[("name", "latch")]),
        ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("b", &[("target", "head")]),
    ];
    let before = stream.len();
    run(&mut stream);
    assert_eq!(
        stream.len(),
        before,
        "PRE never changes the instruction count"
    );
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::Mul,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Ret,
            CodeOp::Label,
            CodeOp::Mul,
            CodeOp::Branch,
        ]
    );
    assert_eq!(stream[0].get("dst").as_deref(), Some("%v3"));
    assert_eq!(mov_source(&stream, "%v5").as_deref(), Some("%v3"));
}

/// Two independent diamonds in one stream are two rewrites, each inserted in
/// its own gap arm.
#[test]
fn two_diamonds_are_completed_independently() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "arm1")]),
        ci("b", &[("target", "join1")]),
        ci("label", &[("name", "arm1")]),
        ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("b", &[("target", "join1")]),
        ci("label", &[("name", "join1")]),
        ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
        ci("b.eq", &[("target", "arm2")]),
        ci("b", &[("target", "join2")]),
        ci("label", &[("name", "arm2")]),
        ci("add", &[("dst", "%v13"), ("lhs", "%v11"), ("rhs", "%v12")]),
        ci("b", &[("target", "join2")]),
        ci("label", &[("name", "join2")]),
        ci("add", &[("dst", "%v14"), ("lhs", "%v11"), ("rhs", "%v12")]),
        ci("mov", &[("dst", "%v15"), ("src", "%v14")]),
        ci("ret", &[]),
    ];
    let before = stream.len();
    run(&mut stream);
    assert_eq!(stream.len(), before);
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Mul,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Mul,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Add,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Add,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::Ret,
        ]
    );
    assert_eq!(mov_source(&stream, "%v5").as_deref(), Some("%v3"));
    assert_eq!(mov_source(&stream, "%v15").as_deref(), Some("%v13"));
}

/// A literal operand is part of the expression's identity and needs no
/// availability proof; the gap arm that opens with a label takes the copy just
/// after its label.
#[test]
fn a_literal_operand_completes_into_a_labelled_gap_arm() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "arm")]),
        ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "7")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "arm")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "7")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Add,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Add,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::Ret,
        ]
    );
    assert_eq!(stream[5].get("rhs").as_deref(), Some("7"));
    assert_eq!(mov_source(&stream, "%v5").as_deref(), Some("%v3"));
}

/// The copy is placed ahead of a flag-setting instruction that ends the gap
/// arm, never between it and the branch that reads its flags.
#[test]
fn the_copy_lands_before_a_flag_setter_that_ends_the_gap_arm() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "arm")]),
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "1")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "arm")]),
        ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
        ci("ret", &[]),
    ];
    run(&mut stream);
    assert_eq!(
        &ops(&stream)[..5],
        &[
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Mul,
            CodeOp::CmpImm,
            CodeOp::Branch
        ]
    );
    assert_eq!(mov_source(&stream, "%v5").as_deref(), Some("%v3"));
}

/// An operand that is a loop-carried value (a φ at the loop header) is
/// available in the gap arm the header dominates.
#[test]
fn a_loop_carried_operand_is_available_below_its_header() {
    let mut stream = vec![
        ci("mov", &[("dst", "%v1"), ("src", "%v7")]),
        ci("b", &[("target", "head")]),
        ci("label", &[("name", "head")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "gap")]),
        ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "gap")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
        ci("add", &[("dst", "%v1"), ("lhs", "%v1"), ("rhs", "1")]),
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
        ci("b.ne", &[("target", "head")]),
        ci("ret", &[]),
    ];
    let before = stream.len();
    run(&mut stream);
    assert_eq!(stream.len(), before);
    // The gap arm (after its label) now computes the product.
    let gap = stream
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("gap"))
        .expect("gap label");
    assert_eq!(stream[gap + 1].op, CodeOp::Mul);
    assert_eq!(stream[gap + 1].get("dst").as_deref(), Some("%v3"));
    assert_eq!(mov_source(&stream, "%v5").as_deref(), Some("%v3"));
}

/// A holder register written twice is not a value the join may read, so the
/// row declines.
#[test]
fn a_twice_written_holder_is_declined() {
    let stream = || {
        vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "arm")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "arm")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("mov", &[("dst", "%v3"), ("src", "%v6")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "join")]),
            ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off);
    assert_eq!(ops(&off), ops(&stream()));
    assert_eq!(mov_source(&off, "%v5").as_deref(), Some("%v4"));
}

/// The preheader defines an operand in the flag-setting run that ends it — the
/// only place the copy could go is above that definition, so the row declines
/// rather than read the operand before it is written.
#[test]
fn an_operand_defined_at_the_insertion_point_is_declined() {
    let stream = || {
        vec![
            ci("subs", &[("dst", "%v1"), ("lhs", "%v7"), ("rhs", "1")]),
            ci("b", &[("target", "head")]),
            ci("label", &[("name", "head")]),
            ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "latch")]),
            ci("ret", &[]),
            ci("label", &[("name", "latch")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b", &[("target", "head")]),
        ]
    };
    let mut off = stream();
    run(&mut off);
    assert_eq!(ops(&off), ops(&stream()));
    assert_eq!(mov_source(&off, "%v5").as_deref(), Some("%v4"));
}

/// The preheader does work of its own before its branch: the moved
/// computation goes after that work and just before the branch, not ahead of
/// it.
#[test]
fn the_inserted_computation_follows_the_gap_block_s_own_work() {
    let mut stream = vec![
        ci("add", &[("dst", "%v8"), ("lhs", "%v10"), ("rhs", "%v11")]),
        ci("b", &[("target", "head")]),
        ci("label", &[("name", "head")]),
        ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "latch")]),
        ci("ret", &[]),
        ci("label", &[("name", "latch")]),
        ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("b", &[("target", "head")]),
    ];
    run(&mut stream);
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::Add,
            CodeOp::Mul,
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Ret,
            CodeOp::Label,
            CodeOp::Mul,
            CodeOp::Branch,
        ]
    );
    assert_eq!(stream[1].get("dst").as_deref(), Some("%v3"));
    assert_eq!(mov_source(&stream, "%v5").as_deref(), Some("%v3"));
}

/// The header itself defines an operand of the recomputation (`%v7`), so the
/// latch's copy reads the same value but the preheader - the gap - runs before
/// that value exists: the row declines instead of inserting a read of `%v7`
/// ahead of its definition.
#[test]
fn an_operand_the_join_itself_defines_is_declined() {
    let stream = || {
        vec![
            ci("b", &[("target", "head")]),
            ci("label", &[("name", "head")]),
            ci("add", &[("dst", "%v7"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("mul", &[("dst", "%v4"), ("lhs", "%v7"), ("rhs", "%v2")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "latch")]),
            ci("ret", &[]),
            ci("label", &[("name", "latch")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v7"), ("rhs", "%v2")]),
            ci("b", &[("target", "head")]),
        ]
    };
    let mut off = stream();
    run(&mut off);
    assert_eq!(ops(&off), ops(&stream()));
    assert_eq!(mov_source(&off, "%v5").as_deref(), Some("%v4"));
}
