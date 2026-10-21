//! Loop fusion (`optimizer::opt1::fuse`) over the shapes lowering actually
//! produces, which the row's inline tests (one shared `i`, constant bounds)
//! do not: per-loop iteration temps (`$for_iterN`) re-bound to the user's
//! variable at the body head, bounds pre-bound into `$for_endN` temps with the
//! second loop's bound bind sitting between the two loops, and a pair nested
//! inside a branch.

use super::nir_build::*;
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirOp, NirValue};

fn fuse(level: u8, body: Vec<NirOp>) -> Vec<NirOp> {
    run_row(level, body, crate::optimizer::opt1::fuse::fuse)
}

fn less(a: &str, b: &str) -> NirValue {
    binary(BinaryOp::Less, local(a), local(b))
}

/// Two lowering-shaped loops over the same constant range fuse: the second
/// bound bind slides in front, the second body's `LET i = $for_iter2` mirror
/// goes (the first body's identical bind serves its reads), and the second
/// temp, still read directly, is bridged from the first.
#[test]
fn lowering_shaped_loops_fuse_through_their_bound_temps() {
    let body = fuse(
        3,
        vec![
            bind("$for_end1", int_const("9")),
            for_loop(
                "$for_iter1",
                int_const("0"),
                local("$for_end1"),
                int_const("1"),
                vec![bind("i", local("$for_iter1")), assign("a", less("i", "p"))],
            ),
            bind("$for_end2", int_const("9")),
            for_loop(
                "$for_iter2",
                int_const("0"),
                local("$for_end2"),
                int_const("1"),
                vec![
                    bind("i", local("$for_iter2")),
                    assign("b", less("i", "q")),
                    assign("c", local("$for_iter2")),
                ],
            ),
        ],
    );
    assert_eq!(body.len(), 3, "two bound binds and one fused loop");
    assert!(matches!(&body[1], NirOp::Bind { name, .. } if name == "$for_end2"));
    let NirOp::For {
        name, body: fused, ..
    } = &body[2]
    else {
        panic!("the fused loop comes last");
    };
    assert_eq!(name, "$for_iter1");
    let shape: Vec<String> = fused
        .iter()
        .map(|op| match op {
            NirOp::Bind {
                name,
                value: Some(NirValue::Local(source)),
                ..
            } => format!("bind {name}={source}"),
            NirOp::Assign { name, .. } => format!("assign {name}"),
            _ => "other".to_string(),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "bind i=$for_iter1",
            "assign a",
            "bind $for_iter2=$for_iter1",
            "assign b",
            "assign c"
        ]
    );
}

/// A fusible pair inside a branch fuses there.
#[test]
fn a_pair_inside_a_branch_fuses() {
    let pair = || {
        vec![
            for_loop(
                "i",
                int_const("0"),
                int_const("9"),
                int_const("1"),
                vec![assign("a", less("i", "p"))],
            ),
            for_loop(
                "i",
                int_const("0"),
                int_const("9"),
                int_const("1"),
                vec![assign("b", less("i", "q"))],
            ),
        ]
    };
    let body = fuse(
        3,
        vec![NirOp::If {
            condition: bool_const(true),
            then_body: pair(),
            else_body: vec![],
        }],
    );
    let NirOp::If { then_body, .. } = &body[0] else {
        panic!("expected the If");
    };
    assert_eq!(then_body.len(), 1, "the branch's two loops are one");
}

/// Distinct loop variables are bridged only when the second one lives
/// entirely inside its loop and the first never appears in the second loop.
#[test]
fn a_loop_variable_used_outside_its_loop_blocks_fusion() {
    let first = || {
        for_loop(
            "i1",
            int_const("0"),
            int_const("9"),
            int_const("1"),
            vec![assign("a", less("i1", "p"))],
        )
    };
    let escaping = fuse(
        3,
        vec![
            first(),
            for_loop(
                "i2",
                int_const("0"),
                int_const("9"),
                int_const("1"),
                vec![assign("b", less("i2", "q"))],
            ),
            assign("z", local("i2")),
        ],
    );
    assert_eq!(escaping.len(), 3, "`i2` is read after its loop");

    let crossing = fuse(
        3,
        vec![
            first(),
            for_loop(
                "i2",
                int_const("0"),
                int_const("9"),
                int_const("1"),
                vec![assign("b", less("i1", "q"))],
            ),
        ],
    );
    assert_eq!(
        crossing.len(),
        2,
        "the second loop names the first's variable"
    );
}
