use super::*;
use crate::optimizer::{with_opt_level, OptLevel};

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.physical_field(k, v);
    }
    inst
}

fn ops(instructions: &[CodeInstruction]) -> Vec<CodeOp> {
    instructions.iter().map(|inst| inst.op).collect()
}

fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(level), || sink(stream, &model));
}

/// A computation used only on the taken side moves there, so the
/// fall-through stops paying for it.
#[test]
fn a_one_sided_computation_sinks_into_its_branch() {
    let mut stream = vec![
        ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
        ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
        ci("b.eq", &[("target", "used")]),
        ci("ret", &[]),
        ci("label", &[("name", "used")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
        ci("ret", &[]),
    ];
    run(&mut stream, 3);
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Ret,
            CodeOp::Label,
            CodeOp::Mul,
            CodeOp::Mov,
            CodeOp::Ret,
        ],
    );
}

/// A computation used on both sides stays where it is.
#[test]
fn a_two_sided_computation_stays() {
    let stream = || {
        vec![
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 3);
    assert_eq!(ops(&off), ops(&stream()));
}

/// A successor with a second predecessor could be reached without running
/// `B`, so nothing sinks into it.
#[test]
fn a_shared_successor_is_not_a_sink_target() {
    let stream = || {
        vec![
            ci("b", &[("target", "used")]),
            ci("label", &[("name", "top")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 3);
    assert_eq!(ops(&off), ops(&stream()));
}

/// A load sinks only when nothing between it and the branch may write
/// memory.
#[test]
fn a_load_does_not_sink_past_a_call() {
    let stream = || {
        vec![
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("bl", &[("target", "_helper")]),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 3);
    assert_eq!(ops(&off), ops(&stream()));
}

/// With the path clear, the load does sink.
#[test]
fn a_load_sinks_when_the_path_is_memory_quiet() {
    let mut stream = vec![
        ci(
            "ldr_u64",
            &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
        ),
        ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
        ci("b.eq", &[("target", "used")]),
        ci("ret", &[]),
        ci("label", &[("name", "used")]),
        ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
        ci("ret", &[]),
    ];
    run(&mut stream, 3);
    assert_eq!(stream[0].op, CodeOp::CmpImm);
    assert_eq!(stream[4].op, CodeOp::LdrU64);
}

/// A destination register written more than once is never moved: the
/// overlay does not make the stream's registers single-assignment.
#[test]
fn a_multiply_defined_destination_stays() {
    let stream = || {
        vec![
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("mov", &[("dst", "%v3"), ("src", "%v1")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 3);
    assert_eq!(ops(&off), ops(&stream()));
}

/// The same load leads both arms of a branch, so it belongs above the
/// branch and the two copies go.
#[test]
fn an_identical_leading_load_is_hoisted() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "other")]),
        ci(
            "ldr_u64",
            &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
        ),
        ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
        ci("ret", &[]),
        ci("label", &[("name", "other")]),
        ci(
            "ldr_u64",
            &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
        ),
        ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
        ci("ret", &[]),
    ];
    run(&mut stream, 3);
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::LdrU64,
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Mov,
            CodeOp::Ret,
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::Ret,
        ],
        "one load above the branch, none in the arms"
    );
}

/// The hoist lands before the flag-setting compare, never between it and
/// the branch that reads it.
#[test]
fn the_hoist_lands_before_the_compare() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "other")]),
        ci(
            "ldr_u64",
            &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
        ),
        ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
        ci("ret", &[]),
        ci("label", &[("name", "other")]),
        ci(
            "ldr_u64",
            &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
        ),
        ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
        ci("ret", &[]),
    ];
    run(&mut stream, 3);
    assert_eq!(stream[0].op, CodeOp::LdrU64);
    assert_eq!(
        stream[1].op,
        CodeOp::CmpImm,
        "the compare still feeds its branch"
    );
    assert_eq!(stream[2].op, CodeOp::BranchEq);
}

/// Different slots are different accesses.
#[test]
fn a_different_slot_is_not_hoisted() {
    let stream = || {
        vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
            ci("label", &[("name", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "16")],
            ),
            ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 3);
    assert_eq!(ops(&off), ops(&stream()));
}

/// An arm reachable from elsewhere would have the access removed from a
/// path the branching block never ran, so the row declines.
#[test]
fn a_shared_arm_is_not_hoisted_from() {
    let stream = || {
        vec![
            ci("b", &[("target", "other")]),
            ci("label", &[("name", "top")]),
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
            ci("label", &[("name", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 3);
    assert_eq!(ops(&off), ops(&stream()));
}

/// An identical store leading both arms is one store.
#[test]
fn an_identical_leading_store_is_hoisted() {
    let mut stream = vec![
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "other")]),
        ci(
            "str_u64",
            &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
        ),
        ci("ret", &[]),
        ci("label", &[("name", "other")]),
        ci(
            "str_u64",
            &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
        ),
        ci("ret", &[]),
    ];
    run(&mut stream, 3);
    assert_eq!(
        ops(&stream),
        vec![
            CodeOp::StrU64,
            CodeOp::CmpImm,
            CodeOp::BranchEq,
            CodeOp::Ret,
            CodeOp::Label,
            CodeOp::Ret,
        ],
    );
}
/// The rows are off below `-O3`.
#[test]
fn level_two_disables_the_rows() {
    let stream = || {
        vec![
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v4"), ("rhs", "0")]),
            ci("b.eq", &[("target", "used")]),
            ci("ret", &[]),
            ci("label", &[("name", "used")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v3")]),
            ci("ret", &[]),
        ]
    };
    let mut off = stream();
    run(&mut off, 2);
    assert_eq!(ops(&off), ops(&stream()));
}
