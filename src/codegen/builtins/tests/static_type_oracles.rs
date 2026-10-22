//! The builder's static type oracles over NIR (`builder_value_semantics/static_eval.rs`):
//! `static_type_name`, `static_operand_type`, `static_type_name_for_fold`,
//! `overload_arg_type`, `thread_runtime_return_type` and the constant folds
//! `local_constant_value` / `static_string_value` / `static_primitive_text`.
//!
//! Each answers "what type does this value have, without lowering it", and each answer
//! gates codegen (the in-place append arm, an overload's code form, a `typeName`
//! fold). A wrong `Some` picks the wrong lowering; a `None` where a type is knowable
//! takes a fallback or refuses valid source. These rows pin the answer for each NIR
//! value form, including the forms the corpus reaches only inside larger shapes.

use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::target::shared::runtime::RuntimeHelper;
use crate::types::ParameterType;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn text(value: &str) -> NirValue {
    constant(ParameterType::String, value)
}

fn call_result(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::CallResult {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn runtime_call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::RuntimeCall {
        helper: RuntimeHelper::Abi,
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn global(name: &str, type_: ParameterType) -> NirValue {
    NirValue::Global {
        name: name.to_string(),
        type_,
    }
}

fn member(target: NirValue, member: &str) -> NirValue {
    NirValue::MemberAccess {
        target: Box::new(target),
        member: member.to_string(),
    }
}

fn result_value(value: NirValue) -> NirValue {
    NirValue::ResultValue {
        value: Box::new(value),
    }
}

/// A harness whose package `pkg.makeResult` returns `Result OF Integer`.
fn harness() -> BuilderHarness<'static> {
    let mut harness = BuilderHarness::default();
    harness.package_return_types.insert(
        "pkg.makeResult".to_string(),
        ParameterType::ResultOf(Box::new(ParameterType::Integer)),
    );
    harness
}

/// A stateful resource handle: `fs.File STATE Integer`.
fn stateful_file() -> ParameterType {
    ParameterType::Stateful {
        base: Box::new(ParameterType::named("fs.File")),
        state: Box::new(ParameterType::Integer),
    }
}

/// The leaf and wrapper forms each carry or imply their own type.
#[test]
fn static_type_name_answers_every_typed_value_form() {
    let harness = harness();
    let platform = TestPlatform;
    let builder = harness.builder("oracle", &platform);
    let integer = constant(ParameterType::Integer, "1");
    let result_of_integer = constant(
        ParameterType::ResultOf(Box::new(ParameterType::Integer)),
        "0",
    );
    let rows: Vec<(&str, NirValue, Option<ParameterType>)> = vec![
        (
            "a by-reference local",
            NirValue::LocalRef {
                name: "n".to_string(),
                type_: ParameterType::Integer,
            },
            Some(ParameterType::Integer),
        ),
        (
            "a closure capture",
            NirValue::Capture {
                index: 0,
                type_: ParameterType::String,
                by_ref: false,
            },
            Some(ParameterType::String),
        ),
        (
            "a union payload extract",
            NirValue::UnionExtract {
                type_: ParameterType::declared("Circle"),
                value: Box::new(integer.clone()),
            },
            Some(ParameterType::declared("Circle")),
        ),
        (
            "a Result's Ok test",
            NirValue::ResultIsOk {
                value: Box::new(result_of_integer.clone()),
            },
            Some(ParameterType::Boolean),
        ),
        (
            "a Result's error",
            NirValue::ResultError {
                value: Box::new(result_of_integer.clone()),
            },
            Some(ParameterType::named("Error")),
        ),
        (
            "a checked value (its success type)",
            NirValue::Checked {
                type_: ParameterType::Float,
                value: Box::new(integer.clone()),
                observe: false,
            },
            Some(ParameterType::Float),
        ),
        (
            "a trapped call of a tabulated builtin",
            call_result("len", vec![text("abc")]),
            Some(ParameterType::Integer),
        ),
        (
            "a Result's value (its success type)",
            result_value(result_of_integer),
            Some(ParameterType::Integer),
        ),
        (
            "a non-Result read as a Result's value (its own type)",
            result_value(integer.clone()),
            Some(ParameterType::Integer),
        ),
        (
            "a NOT",
            NirValue::Unary {
                op: UnaryOp::Not,
                operand: Box::new(constant(ParameterType::Boolean, "true")),
                loc: NirSourceLoc::default(),
            },
            Some(ParameterType::Boolean),
        ),
        (
            "a map entry's unknown member",
            member(
                constant(
                    ParameterType::map_entry_of(ParameterType::String, ParameterType::Integer),
                    "",
                ),
                "size",
            ),
            None,
        ),
    ];
    for (what, value, expected) in rows {
        assert_eq!(builder.static_type_name(&value), expected, "{what}");
    }
}

/// A call the hand-written table does not name is still typed by the WIDER queries:
/// the operand query and the fold query through the registry resolver, the overload
/// query through the package return table, a `Result`'s success type, and a resource's
/// `STATE`.
#[test]
fn the_wider_queries_type_what_static_type_name_leaves_open() {
    let harness = harness();
    let platform = TestPlatform;
    let builder = harness.builder("oracle", &platform);
    let upper = call_result("strings.upper", vec![text("a")]);
    assert_eq!(builder.static_type_name(&upper), None, "not in the table");
    assert_eq!(
        builder.static_operand_type(&upper),
        Some(ParameterType::String)
    );
    assert_eq!(
        builder.static_type_name_for_fold(&upper),
        Some(ParameterType::String)
    );
    assert_eq!(
        builder.overload_arg_type(&upper),
        Some(ParameterType::String)
    );
    assert_eq!(
        builder.overload_arg_type(&result_value(upper.clone())),
        Some(ParameterType::String),
        "a non-Result operand of a Result read keeps its type"
    );
    assert_eq!(
        builder.overload_arg_type(&result_value(call_result("pkg.makeResult", Vec::new()))),
        Some(ParameterType::Integer),
        "a package call's Result answers its success type"
    );
    let file = global("handle", stateful_file());
    assert_eq!(
        builder.overload_arg_type(&member(file.clone(), "state")),
        Some(ParameterType::Integer),
        "a resource's STATE"
    );
    assert_eq!(
        builder.overload_arg_type(&member(file.clone(), "pos")),
        None,
        "a resource has no other member"
    );
    assert_eq!(
        builder.static_type_name_for_fold(&member(file, "state")),
        Some(ParameterType::Integer)
    );
    let unknown = NirValue::Binary {
        op: BinaryOp::Add,
        left: Box::new(NirValue::Local("undeclared".to_string())),
        right: Box::new(constant(ParameterType::Integer, "1")),
        loc: NirSourceLoc::default(),
    };
    assert_eq!(
        builder.static_type_name_for_fold(&unknown),
        None,
        "an operand nothing can type leaves the sum untyped"
    );
}

/// The constant folds: `toString` of a constant through a runtime call, a fold that
/// does not apply to an unlisted `strings` member, and a constant whose type has no
/// text form.
#[test]
fn the_constant_folds_answer_only_what_they_can_fold() {
    let harness = harness();
    let platform = TestPlatform;
    let builder = harness.builder("oracle", &platform);
    let five = constant(ParameterType::Integer, "5");
    let to_string = runtime_call("toString", vec![five.clone()]);
    match builder.local_constant_value(&to_string) {
        Some(NirValue::Const { type_, value }) => {
            assert_eq!((type_, value.as_str()), (ParameterType::String, "5"));
        }
        _ => panic!("toString(5) folds to the String constant \"5\""),
    }
    assert_eq!(
        builder.static_string_value(&to_string),
        Some("5".to_string())
    );
    assert_eq!(
        builder.static_string_value(&call("strings.upper", vec![text("ab")])),
        Some("AB".to_string())
    );
    assert_eq!(
        builder.static_string_value(&call("strings.trim", vec![text(" ab ")])),
        None,
        "only the case/normalization members fold"
    );
    assert_eq!(
        builder.static_primitive_text(&constant(ParameterType::Money, "1.00")),
        None,
        "a Money constant has no folded text"
    );
}

/// A thread op's result type is read off the handle's `ThreadHandle`; an argument that
/// is not the handle the op needs answers `None` rather than an invented type.
#[test]
fn thread_ops_decline_a_handle_of_the_wrong_shape() {
    let harness = harness();
    let platform = TestPlatform;
    let builder = harness.builder("oracle", &platform);
    let worker = global(
        "w",
        ParameterType::thread_handle(
            true,
            ParameterType::Integer,
            ParameterType::Integer,
            ParameterType::String,
        ),
    );
    let number = constant(ParameterType::Integer, "1");
    let not_isolated_entry = NirValue::FunctionRef {
        name: "entry".to_string(),
        type_: ParameterType::Func(
            vec![ParameterType::Integer],
            Box::new(ParameterType::Integer),
            true,
        ),
    };
    for (what, target, args) in [
        (
            "start with an entry whose first parameter is no handle",
            "thread.start",
            vec![not_isolated_entry],
        ),
        (
            "start with no entry at all",
            "thread.start",
            vec![number.clone()],
        ),
        (
            "waitFor on a worker's own view",
            "thread.waitFor",
            vec![worker],
        ),
        (
            "receive on a number",
            "thread.receive",
            vec![number.clone()],
        ),
        (
            "acceptResource on a number",
            "thread.acceptResource",
            vec![number],
        ),
    ] {
        assert_eq!(
            builder.thread_runtime_return_type(target, &args),
            None,
            "{what}"
        );
    }
}
