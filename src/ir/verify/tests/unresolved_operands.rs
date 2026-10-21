//! An operand whose type `ir::verify` cannot infer — a local nothing binds, in
//! hand-built (package-path) IR — gives the rule that wanted its type nothing
//! to judge, so that rule reports nothing: an `IF` condition, a returned value,
//! a constructor of a type with no field list, a record constructor's field
//! argument, a union extraction's source (or a variant with no name) and a
//! closure capture each skip their own check. (A local nothing binds is the
//! NIR resolution check's to report, not this pass's.)
//!
//! Also on the package path: a builtin's arguments, checked without a source
//! checker in front —
//!
//! - a general built-in predicate in a `collections` callback position whose
//!   derived type the member rejects (`forEach` wants a `Nothing` callback), or
//!   whose collection is not a list, is an argument mismatch naming it;
//! - a `thread.start` entry that is not an `ISOLATED FUNC` is refused.

use super::*;

fn ghost() -> IrValue {
    IrValue::Local("ghost".to_string())
}

fn eval(value: IrValue) -> IrOp {
    IrOp::Eval {
        value,
        loc: IrSourceLoc::default(),
    }
}

fn call(target: &str, args: Vec<IrValue>) -> IrOp {
    eval(IrValue::Call {
        target: target.to_string(),
        args,
        type_: ParameterType::Nothing,
        loc: IrSourceLoc::default(),
    })
}

fn local(name: &str) -> IrValue {
    IrValue::Local(name.to_string())
}

/// `run` with `params`, evaluating `op` and returning nothing.
fn run_with(params: Vec<IrParam>, op: IrOp) -> IrFunction {
    func_returns("run", "Nothing", params, vec![op, ret_none()])
}

fn all_details(project: &IrProject) -> Vec<String> {
    collect_diagnostics(project)
        .into_iter()
        .map(|d| format!("{} {}", d.rule, d.detail))
        .collect()
}

fn shape_types() -> Vec<IrType> {
    vec![
        record_typed("Dot", &[("x", "Integer")]),
        union("Shape", &["Dot"]),
    ]
}

/// Each position, with an operand nothing binds (or, for an extraction, a
/// variant with no name to look up).
fn unresolved_cases() -> Vec<(&'static str, IrFunction, Vec<IrType>)> {
    vec![
        (
            "an IF condition",
            run_with(
                vec![],
                IrOp::If {
                    condition: ghost(),
                    then_body: vec![],
                    else_body: vec![],
                    loc: IrSourceLoc::default(),
                },
            ),
            vec![],
        ),
        (
            "a returned value",
            func_returns("run", "Integer", vec![], vec![ret(ghost())]),
            vec![],
        ),
        (
            "a constructor of a type with no field list",
            run_with(
                vec![],
                eval(IrValue::Constructor {
                    type_: ParameterType::parse("Ghost"),
                    args: vec![],
                }),
            ),
            vec![],
        ),
        (
            "a record constructor's field argument",
            run_with(
                vec![],
                eval(IrValue::Constructor {
                    type_: ParameterType::parse("Point"),
                    args: vec![ghost()],
                }),
            ),
            vec![record_typed("Point", &[("x", "Integer")])],
        ),
        (
            "a union extraction's source",
            run_with(
                vec![],
                eval(IrValue::UnionExtract {
                    type_: ParameterType::parse("Dot"),
                    value: Box::new(ghost()),
                }),
            ),
            shape_types(),
        ),
        (
            "a union extraction of a variant with no name",
            run_with(
                vec![param("s", "Shape", None)],
                eval(IrValue::UnionExtract {
                    type_: ParameterType::named(""),
                    value: Box::new(local("s")),
                }),
            ),
            shape_types(),
        ),
        (
            "a closure capture",
            run_with(
                vec![],
                eval(IrValue::Closure {
                    name: "$lambda0".to_string(),
                    type_: ParameterType::parse("FUNC() AS Nothing"),
                    captures: vec![ghost()],
                }),
            ),
            vec![],
        ),
    ]
}

#[test]
fn an_operand_whose_type_cannot_be_inferred_is_not_judged() {
    for (what, f, types) in unresolved_cases() {
        assert_eq!(
            all_details(&project(vec![f], types)),
            Vec::<String>::new(),
            "{what}"
        );
    }
}

#[test]
fn package_path_builtin_calls_are_checked_without_a_source_checker() {
    let non_isolated_entry = IrValue::FunctionRef {
        name: "work".to_string(),
        type_: ParameterType::Func(
            vec![ParameterType::Integer],
            Box::new(ParameterType::Integer),
            false,
        ),
    };
    let cases = [
        (
            run_with(
                vec![param("xs", "List OF String", None)],
                call("collections.forEach", vec![local("xs"), local("isEmpty")]),
            ),
            "TYPE_CALL_ARGUMENT_MISMATCH Call to `collections.forEach` has argument type(s) \
             (List OF String, FUNC(String) AS Boolean), expected List OF T, FUNC(T) AS Nothing.",
        ),
        (
            run_with(
                vec![param("n", "Integer", None)],
                call("collections.filter", vec![local("n"), local("isEmpty")]),
            ),
            "TYPE_CALL_ARGUMENT_MISMATCH Call to `collections.filter` has argument type(s) \
             (Integer, isEmpty), expected List OF T, FUNC(T) AS Boolean.",
        ),
        (
            run_with(
                vec![],
                call(
                    "thread.start",
                    vec![non_isolated_entry, const_of("Integer", "1")],
                ),
            ),
            "TYPE_CALL_ARGUMENT_MISMATCH thread.start entry point must name an ISOLATED FUNC.",
        ),
    ];
    for (f, want) in cases {
        assert_eq!(all_details(&project(vec![f], vec![])), [want.to_string()]);
    }
}
