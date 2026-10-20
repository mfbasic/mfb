use super::super::local_rewrites::testutil::*;
use super::*;
use crate::operators::BinaryOp;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::{NirParam, NirSourceLoc};

fn loc(line: u32) -> NirSourceLoc {
    NirSourceLoc {
        line,
        column: 3,
        file: 0,
    }
}

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

fn binary(op: BinaryOp, left: NirValue, right: NirValue, line: u32) -> NirValue {
    NirValue::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
        loc: loc(line),
    }
}

fn call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args,
        loc: loc(99),
    }
}

fn function(
    name: &str,
    file: &str,
    params: &[(&str, ParameterType)],
    returns: ParameterType,
    body: Vec<NirOp>,
) -> NirFunction {
    NirFunction {
        name: name.to_string(),
        visibility: "public".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params: params
            .iter()
            .map(|(name, type_)| NirParam {
                name: name.to_string(),
                type_: type_.clone(),
                default: None,
            })
            .collect(),
        returns,
        body,
        file: file.to_string(),
        resource_owners: HashMap::new(),
    }
}

/// `gi(x, y) = x * 29 + y`, written in `grid.mfb` at line 38.
fn gi(file: &str) -> NirFunction {
    function(
        "gi",
        file,
        &[("x", ParameterType::Integer), ("y", ParameterType::Integer)],
        ParameterType::Integer,
        vec![NirOp::Return {
            value: Some(binary(
                BinaryOp::Add,
                binary(BinaryOp::Multiply, local("x"), int_const("29"), 38),
                local("y"),
                38,
            )),
        }],
    )
}

/// A caller in `file` binding `i` and `j` as Integers and returning
/// `value`.
fn caller(file: &str, value: NirValue) -> NirFunction {
    function(
        "caller",
        file,
        &[("i", ParameterType::Integer)],
        ParameterType::Integer,
        vec![
            NirOp::Bind {
                mutable: false,
                name: "j".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("4")),
            },
            NirOp::Return { value: Some(value) },
        ],
    )
}

fn run(module: &mut NirModule, level: u8) {
    with_opt_level(OptLevel(level), || inline_leaf_calls(module));
}

fn returned(module: &NirModule) -> NirValue {
    match &module.functions[1].body[1] {
        NirOp::Return { value: Some(value) } => value.clone(),
        _ => panic!("expected the caller's return"),
    }
}

/// The row's core: the call becomes the helper's expression with each
/// parameter replaced by its argument — here in swapped order, so a naive
/// sequential rename would be caught.
#[test]
fn a_leaf_call_becomes_its_expression() {
    let mut module = test_module(vec![
        gi("grid.mfb"),
        caller("grid.mfb", call("gi", vec![local("j"), local("i")])),
    ]);
    run(&mut module, 1);
    let NirValue::Binary {
        op: BinaryOp::Add,
        left,
        right,
        loc,
    } = returned(&module)
    else {
        panic!("expected the inlined add");
    };
    assert_eq!((loc.line, loc.file), (38, 0), "same file: no file tag");
    assert!(matches!(*right, NirValue::Local(ref name) if name == "i"));
    let NirValue::Binary { left: x, .. } = *left else {
        panic!("expected the inlined multiply");
    };
    assert!(matches!(*x, NirValue::Local(ref name) if name == "j"));
}

/// Across files, every located node of the inlined expression names the
/// helper's file, so its errors report where they were written.
#[test]
fn a_cross_file_inline_tags_the_helpers_file() {
    let mut module = test_module(vec![
        gi("src/grid.mfb"),
        caller(
            "src/rooms.mfb",
            call("gi", vec![local("i"), int_const("7")]),
        ),
    ]);
    run(&mut module, 1);
    let NirValue::Binary { loc, left, .. } = returned(&module) else {
        panic!("expected the inlined add");
    };
    let grid = crate::target::shared::nir::source_file_id("src/grid.mfb");
    assert_eq!(loc.file, grid);
    let NirValue::Binary { loc: inner, .. } = *left else {
        panic!("expected the inlined multiply");
    };
    assert_eq!(inner.file, grid);
    assert_eq!(
        crate::target::shared::nir::source_file_name(grid).as_deref(),
        Some("src/grid.mfb")
    );
}

/// Everything the module docs exclude keeps its call.
#[test]
fn ineligible_calls_are_left_alone() {
    let non_atomic = call(
        "gi",
        vec![
            binary(BinaryOp::Add, local("i"), int_const("1"), 5),
            local("j"),
        ],
    );
    let mistyped = call(
        "gi",
        vec![
            NirValue::Const {
                type_: ParameterType::Byte,
                value: "1".to_string(),
            },
            local("j"),
        ],
    );
    let unknown_local = call("gi", vec![local("nowhere"), local("j")]);
    let short = call("gi", vec![local("i")]);
    for value in [non_atomic, mistyped, unknown_local, short] {
        let mut module = test_module(vec![gi("grid.mfb"), caller("grid.mfb", value)]);
        run(&mut module, 1);
        assert!(matches!(returned(&module), NirValue::Call { .. }));
    }
}

/// A call whose target names a local (a `FUNC`-typed parameter sharing the
/// module function's name) is an indirect call and keeps it — the
/// `rt-behavior/general/typename_user_call` shape.
#[test]
fn a_call_through_a_same_named_local_is_left_alone() {
    let mut through_param = caller("grid.mfb", call("gi", vec![local("i"), local("j")]));
    through_param.params.push(NirParam {
        name: "gi".to_string(),
        type_: ParameterType::Unknown,
        default: None,
    });
    let mut module = test_module(vec![gi("grid.mfb"), through_param]);
    run(&mut module, 1);
    assert!(matches!(returned(&module), NirValue::Call { .. }));
}

/// A callee that is not a scalar one-expression leaf is never inlined.
#[test]
fn non_leaf_callees_are_left_alone() {
    let helper_calling_user_code = function(
        "gi",
        "grid.mfb",
        &[("x", ParameterType::Integer)],
        ParameterType::Integer,
        vec![NirOp::Return {
            value: Some(call("caller", vec![local("x")])),
        }],
    );
    let two_statements = function(
        "gi",
        "grid.mfb",
        &[("x", ParameterType::Integer)],
        ParameterType::Integer,
        vec![
            NirOp::Eval { value: local("x") },
            NirOp::Return {
                value: Some(local("x")),
            },
        ],
    );
    let string_result = function(
        "gi",
        "grid.mfb",
        &[("x", ParameterType::Integer)],
        ParameterType::String,
        vec![NirOp::Return {
            value: Some(NirValue::Const {
                type_: ParameterType::String,
                value: "s".to_string(),
            }),
        }],
    );
    let mut isolated = gi("grid.mfb");
    isolated.isolated = true;
    // A body the optimizer synthesized (kind `function`) is not a user `func`.
    let mut synthesized = gi("grid.mfb");
    synthesized.kind = "function".to_string();
    for callee in [
        helper_calling_user_code,
        two_statements,
        string_result,
        isolated,
        synthesized,
    ] {
        let args = callee.params.iter().map(|_| local("i")).collect();
        let mut module = test_module(vec![callee, caller("grid.mfb", call("gi", args))]);
        run(&mut module, 1);
        assert!(matches!(returned(&module), NirValue::Call { .. }));
    }
}

/// A call the inline-`TRAP` desugar wraps (`CallResult`), or one sitting
/// under an error capture, is left alone.
#[test]
fn calls_under_an_error_capture_are_left_alone() {
    let captured = NirValue::Checked {
        type_: ParameterType::Integer,
        value: Box::new(call("gi", vec![local("i"), local("j")])),
        observe: true,
    };
    let mut module = test_module(vec![gi("grid.mfb"), caller("grid.mfb", captured)]);
    run(&mut module, 1);
    let NirValue::Checked { value, .. } = returned(&module) else {
        panic!("expected the capture");
    };
    assert!(matches!(*value, NirValue::Call { .. }));

    let result = NirValue::CallResult {
        target: "gi".to_string(),
        args: vec![local("i"), local("j")],
        loc: loc(1),
    };
    let mut module = test_module(vec![gi("grid.mfb"), caller("grid.mfb", result)]);
    run(&mut module, 1);
    assert!(matches!(returned(&module), NirValue::CallResult { .. }));
}

/// Off at `-O0`.
#[test]
fn the_row_is_off_at_level_zero() {
    let mut module = test_module(vec![
        gi("grid.mfb"),
        caller("grid.mfb", call("gi", vec![local("i"), local("j")])),
    ]);
    run(&mut module, 0);
    assert!(matches!(returned(&module), NirValue::Call { .. }));
}
