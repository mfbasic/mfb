use super::testutil::*;
use super::*;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::NirFunction;

/// Run the driver over `body` as a one-function module at the given dial
/// level, returning the rewritten body.
fn apply_body(body: Vec<NirOp>, level: u8) -> Vec<NirOp> {
    let function = NirFunction {
        name: "f".to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners: HashMap::new(),
    };
    let mut module = test_module(vec![function]);
    with_opt_level(OptLevel(level), || apply(&mut module));
    module.functions.remove(0).body
}

/// Folding feeds the sibling rows within one node visit: `(1+1) * x` folds
/// its left child to `2`, and the parent then strength-reduces to `x + x`.
#[test]
fn folded_constants_feed_strength_reduction() {
    let ops = apply_body(
        vec![
            NirOp::Bind {
                mutable: false,
                name: "x".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("5")),
            },
            NirOp::Eval {
                value: binary(
                    BinaryOp::Multiply,
                    binary(BinaryOp::Add, int_const("1"), int_const("1")),
                    local("x"),
                ),
            },
        ],
        1,
    );
    let NirOp::Eval { value } = &ops[1] else {
        panic!("expected Eval");
    };
    assert_eq!(shape(value), "(local(x) + local(x))");
}

/// Both rows compose in one bottom-up walk: the inner `x * 1` falls to the
/// algebraic row, then the exposed `x * 2` falls to the strength row.
#[test]
fn rows_compose_bottom_up() {
    let ops = apply_body(
        vec![
            NirOp::Bind {
                mutable: false,
                name: "x".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("5")),
            },
            NirOp::Eval {
                value: binary(
                    BinaryOp::Multiply,
                    binary(BinaryOp::Multiply, local("x"), int_const("1")),
                    int_const("2"),
                ),
            },
        ],
        1,
    );
    let NirOp::Eval { value } = &ops[1] else {
        panic!("expected Eval");
    };
    assert_eq!(shape(value), "(local(x) + local(x))");
}

/// A shadowing rebind in a nested body must not leak its type outward, and
/// the inner body must see the inner type.
#[test]
fn shadowed_bindings_use_the_innermost_type() {
    let ops = apply_body(
        vec![
            NirOp::Bind {
                mutable: false,
                name: "x".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("5")),
            },
            NirOp::If {
                condition: local("c"),
                then_body: vec![
                    NirOp::Bind {
                        mutable: false,
                        name: "x".to_string(),
                        type_: ParameterType::Float,
                        value: None,
                    },
                    // Inner x is Float: the Integer identity must not fire.
                    NirOp::Eval {
                        value: binary(BinaryOp::Add, local("x"), int_const("0")),
                    },
                ],
                else_body: vec![],
            },
            // Outer x is Integer again: the identity fires.
            NirOp::Eval {
                value: binary(BinaryOp::Add, local("x"), int_const("0")),
            },
        ],
        1,
    );
    let NirOp::If { then_body, .. } = &ops[1] else {
        panic!("expected If");
    };
    let NirOp::Eval { value } = &then_body[1] else {
        panic!("expected Eval");
    };
    assert_eq!(shape(value), "(local(x) + const(0))");
    let NirOp::Eval { value } = &ops[2] else {
        panic!("expected Eval");
    };
    assert_eq!(shape(value), "local(x)");
}

/// plan-100 §3(a): at `-O0` the rows are a no-op on values they rewrite at
/// `-O1` — shared input so a silently-dead row fails the `-O1` half.
#[test]
fn level_zero_disables_the_rows() {
    let body = || {
        vec![
            NirOp::Bind {
                mutable: false,
                name: "x".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("5")),
            },
            NirOp::Return {
                value: Some(binary(
                    BinaryOp::Add,
                    binary(BinaryOp::Multiply, local("x"), int_const("1")),
                    binary(BinaryOp::Multiply, local("x"), int_const("2")),
                )),
            },
        ]
    };
    let off = apply_body(body(), 0);
    let NirOp::Return { value: Some(value) } = &off[1] else {
        panic!("expected Return");
    };
    assert_eq!(
        shape(value),
        "((local(x) * const(1)) + (local(x) * const(2)))",
        "-O0 must not rewrite"
    );

    let on = apply_body(body(), 1);
    let NirOp::Return { value: Some(value) } = &on[1] else {
        panic!("expected Return");
    };
    assert_eq!(
        shape(value),
        "(local(x) + (local(x) + local(x)))",
        "-O1 must run both rows"
    );
}
