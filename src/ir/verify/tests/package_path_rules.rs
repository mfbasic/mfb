//! Rules `ir::verify` enforces only on the PACKAGE path — decoded `.mfp` IR,
//! where nothing upstream wrote the shape and a crafted package is the threat
//! model — driven with hand-built IR through `collect_diagnostics`:
//!
//! - statement nesting past `MAX_IR_NESTING_DEPTH` is refused rather than
//!   recursed into;
//! - a `Checked` value wrapping a call is refused (a call's error does not join
//!   the capture, so the desugar always lifts calls out first);
//! - a call through a function VALUE must supply exactly its callable type's
//!   parameter count (on the source path that count is `ir::shape`'s);
//! - the dead-inline-handler rule judges only the desugar's own
//!   `resultIsOk($trap_resN)` shape, never another `IF` on a result.

use super::*;

fn call(target: &str, args: Vec<IrValue>, ty: &str) -> IrValue {
    IrValue::Call {
        target: target.to_string(),
        args,
        type_: ParameterType::parse(ty),
        loc: IrSourceLoc::default(),
    }
}

fn local(name: &str) -> IrValue {
    IrValue::Local(name.to_string())
}

fn if_op(condition: IrValue, then_body: Vec<IrOp>) -> IrOp {
    IrOp::If {
        condition,
        then_body,
        else_body: vec![],
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn statement_nesting_past_the_limit_is_refused() {
    let mut body = vec![ret(const_of("Integer", "0"))];
    for _ in 0..(crate::ir::MAX_IR_NESTING_DEPTH + 2) {
        body = vec![if_op(const_of("Boolean", "true"), body)];
    }
    body.push(ret(const_of("Integer", "0")));
    let diagnostics = collect_diagnostics(&project(vec![func("deep", vec![], body)], vec![]));
    assert!(
        diagnostics
            .iter()
            .any(|d| d.rule == super::super::VERIFY_TYPE
                && d.detail
                    .contains("statement nesting exceeds the 256 level limit")),
        "{:?}",
        diagnostics.iter().map(|d| &d.detail).collect::<Vec<_>>()
    );
}

#[test]
fn a_checked_value_wrapping_a_call_is_refused() {
    let helper = func("helper", vec![], vec![ret(const_of("Integer", "1"))]);
    let main = func(
        "main",
        vec![],
        vec![
            bind(
                "r",
                "Integer",
                Some(IrValue::Checked {
                    type_: ParameterType::Integer,
                    value: Box::new(binary(
                        BinaryOp::Add,
                        call("helper", vec![], "Integer"),
                        const_of("Integer", "1"),
                        "Integer",
                    )),
                    observe: true,
                }),
                false,
                false,
            ),
            ret(local("r")),
        ],
    );
    let diagnostics = collect_diagnostics(&project(vec![helper, main], vec![]));
    assert!(
        diagnostics
            .iter()
            .any(|d| d.detail.contains("Checked wraps a call to `helper`")),
        "{:?}",
        diagnostics.iter().map(|d| &d.detail).collect::<Vec<_>>()
    );
}

#[test]
fn a_function_value_call_must_supply_its_parameter_count() {
    let main = func(
        "main",
        vec![param("f", "FUNC(Integer) AS Integer", None)],
        vec![ret(call(
            "f",
            vec![const_of("Integer", "1"), const_of("Integer", "2")],
            "Integer",
        ))],
    );
    let diagnostics = collect_diagnostics(&project(vec![main], vec![]));
    assert!(
        diagnostics
            .iter()
            .any(|d| d.rule == "TYPE_CALL_ARITY_MISMATCH"
                && d.detail == "Call to `f` has 2 argument(s), expected 1."),
        "{:?}",
        diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.rule, d.detail))
            .collect::<Vec<_>>()
    );
}

/// A `$`-temp bound to a value that cannot fail, then `IF resultIsOk(temp)`.
fn handler_test(temp: &str) -> IrProject {
    let main = func(
        "main",
        vec![param("a", "Integer", None)],
        vec![
            bind(
                temp,
                "#Result OF Integer",
                Some(IrValue::Checked {
                    type_: ParameterType::Integer,
                    value: Box::new(local("a")),
                    observe: true,
                }),
                false,
                false,
            ),
            // `resultIsOk` of a non-local is never the desugar's test either.
            if_op(
                IrValue::ResultIsOk {
                    value: Box::new(call("make", vec![], "#Result OF Integer")),
                },
                vec![],
            ),
            if_op(
                IrValue::ResultIsOk {
                    value: Box::new(local(temp)),
                },
                vec![],
            ),
            ret(const_of("Integer", "0")),
        ],
    );
    project(vec![main], vec![])
}

#[test]
fn only_the_inline_trap_desugar_shape_is_judged_for_a_dead_handler() {
    // The desugar's own temp over a value that cannot fail: a dead handler.
    expect_rule(&handler_test("$trap_res1"), "TYPE_INLINE_TRAP_DEAD_HANDLER");
    // The identical shape under any other `$`-temp is not a trap at all.
    expect_no_rule(&handler_test("$other1"), "TYPE_INLINE_TRAP_DEAD_HANDLER");
}
