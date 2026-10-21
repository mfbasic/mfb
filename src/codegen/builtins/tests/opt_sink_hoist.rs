//! The hoisting half of code sinking/hoisting (`optimizer::opt2::sink`): an
//! identical `sp`-slot access leading both arms of a branch moves up into the
//! branching block, but only when moving it is provably invisible.
//!
//! The row's own tests cover the hoist and its slot/arm refusals; these cover
//! the remaining conditions — a store whose value register is written more
//! than once (unstable), a load whose destination is written by a third
//! instruction, a load whose destination is read where neither arm dominates
//! — and a branching block that opens with a label, where the hoist lands
//! after the label.

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
        crate::optimizer::opt2::sink::sink(stream, &model)
    });
}

fn load(dst: &'static str) -> CodeInstruction {
    ci("ldr_u64", &[("dst", dst), ("base", "sp"), ("offset", "8")])
}

fn store(src: &'static str) -> CodeInstruction {
    ci("str_u64", &[("src", src), ("base", "sp"), ("offset", "8")])
}

/// A diamond whose two arms open with `access`; `before` precedes the compare
/// and `after_left`/`after_right` follow the access in each arm.
fn diamond(
    before: Vec<CodeInstruction>,
    access: fn() -> CodeInstruction,
    after_left: Vec<CodeInstruction>,
    after_right: Vec<CodeInstruction>,
) -> Vec<CodeInstruction> {
    let mut stream = before;
    stream.push(ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]));
    stream.push(ci("b.eq", &[("target", "other")]));
    stream.push(access());
    stream.extend(after_left);
    stream.push(ci("label", &[("name", "other")]));
    stream.push(access());
    stream.extend(after_right);
    stream
}

#[test]
fn a_store_of_a_twice_written_register_is_not_hoisted() {
    let shape = || {
        diamond(
            vec![
                ci(
                    "mov_imm",
                    &[("dst", "%v7"), ("type", "Integer"), ("value", "1")],
                ),
                ci(
                    "mov_imm",
                    &[("dst", "%v7"), ("type", "Integer"), ("value", "2")],
                ),
            ],
            || store("%v7"),
            vec![ci("ret", &[])],
            vec![ci("ret", &[])],
        )
    };
    let mut stream = shape();
    run(&mut stream);
    assert_eq!(ops(&stream), ops(&shape()));
}

#[test]
fn a_load_whose_destination_has_a_third_writer_is_not_hoisted() {
    let shape = || {
        diamond(
            vec![],
            || load("%v3"),
            vec![ci("mov", &[("dst", "%v5"), ("src", "%v3")]), ci("ret", &[])],
            vec![
                ci(
                    "mov_imm",
                    &[("dst", "%v3"), ("type", "Integer"), ("value", "4")],
                ),
                ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
                ci("ret", &[]),
            ],
        )
    };
    let mut stream = shape();
    run(&mut stream);
    assert_eq!(ops(&stream), ops(&shape()));
}

#[test]
fn a_load_read_where_neither_arm_dominates_is_not_hoisted() {
    let shape = || {
        diamond(
            vec![],
            || load("%v3"),
            vec![ci("b", &[("target", "join")])],
            vec![
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "join")]),
                ci("mov", &[("dst", "%v6"), ("src", "%v3")]),
                ci("ret", &[]),
            ],
        )
    };
    let mut stream = shape();
    run(&mut stream);
    assert_eq!(ops(&stream), ops(&shape()));
}

#[test]
fn a_hoist_into_a_labelled_block_lands_after_its_label() {
    let mut stream = vec![
        ci("b", &[("target", "top")]),
        ci("label", &[("name", "top")]),
    ];
    stream.extend(diamond(
        vec![],
        || load("%v3"),
        vec![ci("mov", &[("dst", "%v5"), ("src", "%v3")]), ci("ret", &[])],
        vec![ci("mov", &[("dst", "%v6"), ("src", "%v3")]), ci("ret", &[])],
    ));
    run(&mut stream);
    assert_eq!(
        ops(&stream)[..5],
        [
            CodeOp::Branch,
            CodeOp::Label,
            CodeOp::LdrU64,
            CodeOp::CmpImm,
            CodeOp::BranchEq
        ],
        "one load, after the block's label and before its compare"
    );
    assert_eq!(
        ops(&stream)
            .iter()
            .filter(|op| **op == CodeOp::LdrU64)
            .count(),
        1
    );
}
