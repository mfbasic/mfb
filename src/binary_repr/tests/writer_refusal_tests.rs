// ---------------------------------------------------------------------------
// writer/functions.rs — a parameter default that is neither a constant nor a
// call to its hidden default function is a lowering defect, refused by name;
// and imported calls are collected through every op and value shape.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;
use crate::ir::{IrParam, IrSourceLoc};

fn integer() -> crate::types::ParameterType {
    crate::types::ParameterType::parse("Integer")
}

/// `pkg` with one export `f(n AS Integer = <default>)`.
fn defaulted(default: IrValue) -> IrProject {
    let mut project = empty_project("pkg");
    let mut function = fn_named("f", "export", "function", "Integer");
    function.params = vec![IrParam {
        name: "n".to_string(),
        type_: integer(),
        default: Some(default),
        loc: IrSourceLoc::default(),
    }];
    project.functions = vec![
        function,
        fn_named("helper", "private", "function", "Integer"),
    ];
    project
}

fn refusal(project: &IrProject) -> String {
    build_binary_repr_bytes(
        project,
        &BinaryReprMetadata::new("pkg".to_string(), "1.0.0".to_string()),
    )
    .map(|_| ())
    .unwrap_err()
}

const PREFIX: &str = "function `f` parameter `n`: a package can store a default only as a \
    constant or a call to its hidden default function, not ";

#[test]
fn a_default_that_is_no_constant_or_hidden_default_call_is_refused() {
    let call = |target: &str| IrValue::Call {
        target: target.to_string(),
        args: vec![],
        loc: IrSourceLoc::default(),
        type_: integer(),
    };
    assert_eq!(
        refusal(&defaulted(call("helper"))),
        format!("{PREFIX}a call to an ordinary function")
    );
    assert_eq!(
        refusal(&defaulted(IrValue::Unary {
            op: crate::operators::UnaryOp::Negate,
            operand: Box::new(const_int("1")),
            type_: integer(),
            loc: IrSourceLoc::default(),
        })),
        format!("{PREFIX}a non-constant value")
    );
    let hidden = crate::internal_name::hidden_default_function_name("f", 0);
    assert_eq!(
        refusal(&defaulted(call(&hidden))),
        format!("{PREFIX}a call to `{hidden}`, which is not in the function table")
    );
}

/// An imported call inside a Set literal is a used symbol, found past a
/// binding with no initializer and a bare RETURN.
#[test]
fn an_imported_call_inside_a_set_literal_is_recorded_as_used() {
    let mut dep = empty_project("dep");
    dep.functions = vec![fn_named("helper", "export", "function", "Integer")];
    let inner = encode_project(
        &dep,
        &BinaryReprMetadata::new("dep".to_string(), "1.0.0".to_string()),
    );
    let dep_path = temp_mfp(&wrap_mfp(&inner, "dep", "dep", "1.0.0"));
    let mut consumer = empty_project("app");
    let mut main = fn_named("main", "export", "function", "Nothing");
    main.body = vec![
        IrOp::Bind {
            mutable: true,
            name: "later".to_string(),
            type_: integer(),
            value: None,
            explicit_type: true,
            loc: IrSourceLoc::default(),
        },
        IrOp::Eval {
            value: IrValue::SetLiteral {
                type_: crate::types::ParameterType::parse("Set OF Integer"),
                values: vec![IrValue::Call {
                    target: "dep.helper".to_string(),
                    args: vec![],
                    loc: IrSourceLoc::default(),
                    type_: integer(),
                }],
            },
            loc: IrSourceLoc::default(),
        },
        IrOp::Return {
            value: None,
            loc: IrSourceLoc::default(),
        },
    ];
    consumer.functions = vec![main];
    let mut metadata = BinaryReprMetadata::new("app".to_string(), "1.0.0".to_string());
    metadata.dependencies = vec![BinaryReprDependency {
        name: "dep".to_string(),
        ident: String::new(),
        version: "1.0.0".to_string(),
        pin: false,
        flags: 0,
    }];
    let lowered = lower_package_project(&consumer, &metadata, std::slice::from_ref(&dep_path))
        .expect("lower package");
    let used = &lowered.imports.entries[0].used_symbols;
    assert_eq!(used.len(), 1);
    assert_eq!(
        string_at(&lowered.strings.values, used[0].name).unwrap(),
        "helper"
    );
    let _ = std::fs::remove_file(&dep_path);
}
