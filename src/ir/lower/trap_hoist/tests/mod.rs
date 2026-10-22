//! `hoist_trap_calls` driven directly on hand-built scrutinee trees.
//!
//! The lift is two walks — `scan_trap_operands` records which nodes are lifted,
//! `rewrite_trap_operands` replaces them — and they must index the same nodes in
//! the same order (the rewrite asserts it). Source lowering only ever hands the
//! desugar a freshly lowered expression, which holds none of the nodes the
//! desugar itself and statement lowering produce (`CallResult`, `Checked`,
//! `UnionExtract`, the three `Result*` accessors), and the parser's 256-level
//! nesting cap keeps the AST shallower than the walks' own depth cap. The two
//! walks still have an arm for each, and these tests hold them to agreeing on
//! every one.

use super::*;
use crate::testutil::concrete_hir_from_src;

const SOURCE: &str = "\
FUNC inner(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(90000001, \"inner failed\")
  RETURN n * 2
END FUNC
";

/// Hoist `root` in a lowering context for [`SOURCE`], in which `inner` is
/// fallible, with a local `n AS Integer` in scope; the rewritten root and the
/// lifted steps.
fn hoist(mut root: IrValue) -> (IrValue, Vec<TrapHoist>) {
    let hir = concrete_hir_from_src(SOURCE);
    let facts = lower_facts(&hir, &HashMap::new(), &[], &[]);
    let mut context = facts.context();
    let mut locals = HashMap::from([("n".to_string(), ParameterType::Integer)]);
    let hoists = hoist_trap_calls(&mut root, &mut locals, &mut context);
    (root, hoists)
}

fn call(target: &str, args: Vec<IrValue>) -> IrValue {
    IrValue::Call {
        target: target.to_string(),
        args,
        type_: ParameterType::Integer,
        loc: IrSourceLoc::default(),
    }
}

/// `inner(n)`: the one fallible call.
fn inner_n() -> IrValue {
    call("inner", vec![IrValue::Local("n".to_string())])
}

/// Builds one wrapper node around its operand.
type Wrap = fn(IrValue) -> IrValue;

/// The single operand a one-operand wrapper node holds.
fn operand(value: &IrValue) -> &IrValue {
    match value {
        IrValue::CallResult { args, .. } => &args[0],
        IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Checked { value, .. } => value,
        _ => panic!("not a one-operand wrapper"),
    }
}

/// The lifted step's call target, which must be a checked `CallResult`.
fn checked_target(hoist: &TrapHoist) -> &str {
    match &hoist.value {
        IrValue::CallResult { target, .. } => target,
        _ => panic!("the lift of a fallible call is a checked `CallResult`"),
    }
}

/// A fallible call under each wrapper kind is found by the scan, lifted by the
/// rewrite as the one checked step, and the wrapper is left reading the lifted
/// local — the two walks agree on every one of these arms.
#[test]
fn a_fallible_call_under_every_wrapper_node_is_lifted_through_it() {
    let int = || ParameterType::Integer;
    let wrappers: [(&str, Wrap); 6] = [
        ("CallResult", |value| IrValue::CallResult {
            target: "take".to_string(),
            args: vec![value],
            type_: ParameterType::Integer,
            loc: IrSourceLoc::default(),
        }),
        ("UnionExtract", |value| IrValue::UnionExtract {
            type_: ParameterType::Integer,
            value: Box::new(value),
        }),
        ("ResultIsOk", |value| IrValue::ResultIsOk {
            value: Box::new(value),
        }),
        ("ResultValue", |value| IrValue::ResultValue {
            type_: ParameterType::Integer,
            value: Box::new(value),
        }),
        ("ResultError", |value| IrValue::ResultError {
            value: Box::new(value),
        }),
        ("Checked", |value| IrValue::Checked {
            type_: ParameterType::Integer,
            value: Box::new(value),
            observe: true,
        }),
    ];
    for (kind, wrap) in wrappers {
        let (root, hoists) = hoist(call("take", vec![wrap(inner_n())]));
        assert_eq!(hoists.len(), 1, "{kind}: one lifted step");
        assert_eq!(checked_target(&hoists[0]), "inner", "{kind}");
        assert_eq!(hoists[0].type_, int(), "{kind}");
        let IrValue::Call { args, .. } = &root else {
            panic!("{kind}: the root stays the outer call");
        };
        match operand(&args[0]) {
            IrValue::Local(name) => assert_eq!(*name, hoists[0].name, "{kind}"),
            _ => panic!("{kind}: the wrapper must read the lifted local"),
        }
    }
}

/// Both walks stop at the same depth cap, so a call nested past it is neither
/// recorded nor rewritten — they still agree, and the shallow fallible call is
/// lifted as usual. (Such a tree never reaches codegen: the package format's
/// 256-level nesting limit refuses it, `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE`.)
#[test]
fn a_call_past_the_walk_depth_cap_is_left_in_place_by_both_walks() {
    let mut deep = inner_n();
    for _ in 0..(TRAP_SCRUTINEE_MAX_DEPTH + 8) {
        deep = IrValue::UnionExtract {
            type_: ParameterType::Integer,
            value: Box::new(deep),
        };
    }
    let (root, hoists) = hoist(call("take", vec![inner_n(), deep]));
    assert_eq!(hoists.len(), 1, "only the shallow call is lifted");
    assert_eq!(checked_target(&hoists[0]), "inner");
    let IrValue::Call { args, .. } = &root else {
        panic!("the root stays the outer call");
    };
    assert!(matches!(&args[0], IrValue::Local(name) if *name == hoists[0].name));
    let mut node = &args[1];
    while let IrValue::UnionExtract { value, .. } = node {
        node = value;
    }
    assert!(
        matches!(node, IrValue::Call { target, .. } if target == "inner"),
        "the deep call is left where it was"
    );
}
