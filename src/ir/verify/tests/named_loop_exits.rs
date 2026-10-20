//! plan-180-C Phase 2: `ir::verify` resolves a named EXIT/CONTINUE by the IR name of
//! the enclosing `FOR`/`FOR EACH` loop, and reports a name no enclosing loop carries as
//! `UNKNOWN_FOR_IDENTIFIER`. The shared builders and fixtures live in `verify/tests/mod.rs`.

use super::*;

fn counted_for(name: &str, body: Vec<IrOp>) -> IrOp {
    IrOp::For {
        name: name.to_string(),
        type_: ParameterType::Integer,
        start: int_const("0"),
        end: int_const("3"),
        step: int_const("1"),
        body,
        loc: IrSourceLoc::default(),
    }
}

fn named_exit(target: &str) -> IrOp {
    IrOp::ExitLoop {
        target: Some(target.to_string()),
        kind: crate::ast::LoopKind::For,
        loc: IrSourceLoc::default(),
    }
}

fn named_continue(target: &str) -> IrOp {
    IrOp::ContinueLoop {
        target: Some(target.to_string()),
        kind: crate::ast::LoopKind::For,
        loc: IrSourceLoc::default(),
    }
}

fn run_with(body: Vec<IrOp>) -> IrProject {
    project(vec![func_returns("run", "Nothing", vec![], body)], vec![])
}

#[test]
fn accepts_named_exit_of_its_own_for() {
    let body = vec![counted_for("i", vec![named_exit("i")])];
    expect_no_rule(&run_with(body), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn accepts_named_exit_through_an_inner_for() {
    let inner = counted_for("j", vec![named_exit("i")]);
    let body = vec![counted_for("i", vec![inner])];
    expect_no_rule(&run_with(body), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn accepts_named_continue_through_an_inner_for() {
    let inner = counted_for("j", vec![named_continue("i")]);
    let body = vec![counted_for("i", vec![inner])];
    expect_no_rule(&run_with(body), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn rejects_named_exit_with_no_enclosing_for_of_that_name() {
    let body = vec![counted_for("i", vec![named_exit("k")])];
    expect_rule(&run_with(body), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn rejects_named_exit_outside_every_loop() {
    expect_rule(&run_with(vec![named_exit("i")]), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn rejects_named_continue_with_no_enclosing_for_of_that_name() {
    let body = vec![counted_for("i", vec![named_continue("k")])];
    expect_rule(&run_with(body), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn rejects_named_exit_naming_a_do_loop() {
    // A DO loop carries no name, so no name can target it.
    let do_loop = IrOp::DoUntil {
        body: vec![named_exit("d")],
        condition: const_of("Boolean", "true"),
        loc: IrSourceLoc::default(),
    };
    expect_rule(&run_with(vec![do_loop]), "UNKNOWN_FOR_IDENTIFIER");
}

#[test]
fn bare_exit_still_matches_by_kind() {
    let body = vec![counted_for(
        "i",
        vec![IrOp::ExitLoop {
            target: None,
            kind: crate::ast::LoopKind::For,
            loc: IrSourceLoc::default(),
        }],
    )];
    expect_no_rule(&run_with(body), "EXIT_NO_MATCHING_LOOP");
}
