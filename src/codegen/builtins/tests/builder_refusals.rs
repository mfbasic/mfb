//! Builder entry points that check what they are handed before emitting a byte, driven
//! directly with the inputs the front end stops: a `String` operator that is not a
//! comparison, a non-`String` concat operand, an enum pattern that names no enum, a
//! list `replace`/`find`/`mid` whose operands disagree with the haystack, `replace` or
//! `find` over a value that is neither a list nor a `String`, an in-place `set` over a
//! type with no collection layout, a `thread::send` without its message, a record
//! built from the wrong number of field slots, and a block of an undeclared type.
//!
//! Each is `ir::verify`'s or the registry's to reject first, so no program reaches
//! them; the alternative to each refusal is a lowering that compares, copies or
//! indexes the wrong bytes. Each row pins the refusal and that it names what was
//! wrong.

use crate::codegen::engine::builder::{CodeBuilder, GlobalValue, ValueResult};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record, union};
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirEnumMember, NirType, NirValue};
use crate::types::ParameterType;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn value_of(builder: &mut CodeBuilder<'_>, type_: ParameterType) -> ValueResult {
    ValueResult {
        origin: None,
        type_,
        location: Operand::from(builder.temporary_vreg()),
        text: "probe".to_string(),
    }
}

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// `ENUM Color { Red }`, `TYPE Pt { x, y AS Integer }`, and `TYPE Holder { shape AS
/// Shape }` over `UNION Shape { Circle }`.
fn colors() -> crate::codegen::engine::builder::TypeModel {
    model(vec![
        record("Pt", vec![field("x", "Integer"), field("y", "Integer")]),
        record("Circle", vec![field("r", "Integer")]),
        union("Shape", vec![("Circle", vec![field("r", "Integer")])]),
        record("Holder", vec![field("shape", "Shape")]),
        NirType {
            kind: "enum".to_string(),
            visibility: "private".to_string(),
            name: "Color".to_string(),
            fields: Vec::new(),
            includes: Vec::new(),
            variants: Vec::new(),
            members: vec![NirEnumMember {
                name: "Red".to_string(),
            }],
        },
    ])
}

/// Run `body` on a fresh builder over a model with `ENUM Color` and the globals
/// `xs AS List OF Integer` and `t AS Thread OF Integer TO Integer`.
fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = colors();
    let mut harness = harness(&model);
    harness
        .string_symbols
        .insert("a".to_string(), "_mfb_str_a".to_string());
    harness.globals.insert(
        "xs".to_string(),
        GlobalValue {
            type_: ParameterType::list_of(ParameterType::Integer),
            offset: 64,
        },
    );
    harness.globals.insert(
        "h".to_string(),
        GlobalValue {
            type_: ParameterType::declared("Holder"),
            offset: 80,
        },
    );
    harness.globals.insert(
        "t".to_string(),
        GlobalValue {
            type_: ParameterType::thread_handle(
                false,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
            offset: 72,
        },
    );
    let platform = TestPlatform;
    let mut builder = harness.builder("refusal", &platform);
    body(&mut builder)
}

fn global(name: &str, type_: ParameterType) -> NirValue {
    NirValue::Global {
        name: name.to_string(),
        type_,
    }
}

#[test]
fn a_string_operator_that_is_not_a_comparison_is_refused() {
    let error = with_builder(|builder| {
        let left = value_of(builder, ParameterType::String);
        let right = value_of(builder, ParameterType::String);
        refused(
            builder.lower_string_comparison_binary(BinaryOp::Add, &left, &right),
            "`+` on two Strings",
        )
    });
    assert!(error.contains("string comparison operator '+'"), "{error}");
}

#[test]
fn a_concat_operand_that_is_not_a_string_is_refused() {
    let error = with_builder(|builder| {
        let number = constant(ParameterType::Integer, "1");
        refused(
            builder.lower_string_concat_chain(&[&number]),
            "an Integer concat operand",
        )
    });
    assert!(
        error.contains("concat operand must be String, got Integer"),
        "{error}"
    );
}

#[test]
fn an_enum_pattern_must_name_an_enum_member() {
    with_builder(|builder| {
        let matched = value_of(builder, ParameterType::declared("Color"));
        let not_a_type = NirValue::MemberAccess {
            target: Box::new(constant(ParameterType::Integer, "1")),
            member: "Red".to_string(),
        };
        let error = refused(
            builder.lower_match_compare(&matched, &not_a_type, "hit"),
            "a member read off a constant as a pattern",
        );
        assert!(error.contains("must name enum type"), "{error}");
        let no_member = NirValue::MemberAccess {
            target: Box::new(NirValue::Local("Color".to_string())),
            member: "Purple".to_string(),
        };
        let error = refused(
            builder.lower_match_compare(&matched, &no_member, "hit"),
            "an enum member that does not exist",
        );
        assert!(
            error.contains("enum member 'Color.Purple' does not resolve"),
            "{error}"
        );
    });
}

/// A record pattern is matched field by field; a field with no comparison (a union)
/// is refused, and the refusal reaches the caller.
#[test]
fn a_record_pattern_with_an_incomparable_field_is_refused() {
    let error = with_builder(|builder| {
        let matched = value_of(builder, ParameterType::declared("Holder"));
        let pattern = global("h", ParameterType::declared("Holder"));
        refused(
            builder.lower_match_compare(&matched, &pattern, "hit"),
            "a record pattern holding a union field",
        )
    });
    assert!(error.contains("'Shape'"), "{error}");
}

#[test]
fn replace_refuses_operands_that_disagree_with_the_value() {
    let list = || global("xs", ParameterType::list_of(ParameterType::Integer));
    let int = || constant(ParameterType::Integer, "1");
    let text = || constant(ParameterType::String, "a");
    for (what, args, expected) in [
        (
            "a String `old` for a List OF Integer",
            vec![list(), text(), int()],
            "list replace old must be Integer, got String",
        ),
        (
            "a String `new` for a List OF Integer",
            vec![list(), int(), text()],
            "list replace new must be Integer, got String",
        ),
        (
            "an Integer value",
            vec![int(), text(), text()],
            "string replace value must be String, got Integer",
        ),
    ] {
        let error = with_builder(|builder| refused(builder.lower_replace(&args), what));
        assert!(error.contains(expected), "{what}: {error}");
    }
}

#[test]
fn find_refuses_operands_that_disagree_with_the_haystack() {
    let list = || global("xs", ParameterType::list_of(ParameterType::Integer));
    let int = || constant(ParameterType::Integer, "1");
    let text = || constant(ParameterType::String, "a");
    for (what, args, expected) in [
        (
            "a String start in a list",
            vec![list(), int(), text()],
            "list find start must be Integer, got String",
        ),
        (
            "a String needle in a List OF Integer",
            vec![list(), text()],
            "list find needle must be Integer or List OF Integer, got String",
        ),
        (
            "an Integer haystack",
            vec![int(), text()],
            "string find haystack must be String, got Integer",
        ),
        (
            "an Integer needle in a String",
            vec![text(), int()],
            "string find needle must be String, got Integer",
        ),
        (
            "a String start in a String",
            vec![text(), text(), text()],
            "string find start must be Integer, got String",
        ),
    ] {
        let error = with_builder(|builder| refused(builder.lower_find(&args), what));
        assert!(error.contains(expected), "{what}: {error}");
    }
}

#[test]
fn mid_refuses_a_list_bound_that_is_not_an_integer() {
    let list = || global("xs", ParameterType::list_of(ParameterType::Integer));
    let int = || constant(ParameterType::Integer, "1");
    let text = || constant(ParameterType::String, "a");
    for (what, args, expected) in [
        (
            "a String start",
            vec![list(), text(), int()],
            "list mid start must be Integer, got String",
        ),
        (
            "a String count",
            vec![list(), int(), text()],
            "list mid count must be Integer, got String",
        ),
    ] {
        let error = with_builder(|builder| refused(builder.lower_mid(&args), what));
        assert!(error.contains(expected), "{what}: {error}");
    }
}

#[test]
fn an_in_place_set_needs_a_collection_layout() {
    let error = with_builder(|builder| {
        let slot = builder.allocate_stack_object("probe", 8);
        refused(
            builder.lower_list_set_in_place(
                slot,
                slot,
                slot,
                &ParameterType::Integer,
                &ParameterType::Integer,
            ),
            "an in-place set over an Integer",
        )
    });
    assert!(
        error.contains("collection type 'Integer' is not supported"),
        "{error}"
    );
}

#[test]
fn a_thread_send_needs_its_handle_and_message() {
    let error = with_builder(|builder| {
        let handle = global(
            "t",
            ParameterType::thread_handle(
                false,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
        );
        refused(
            builder.emit_thread_send_runtime_helper_call(
                "thread.send",
                "_mfb_rt_thread_send",
                &[handle],
                &ParameterType::Nothing,
                false,
            ),
            "a send with no message",
        )
    });
    assert!(error.contains("expects a handle and message"), "{error}");
    let error = with_builder(|builder| {
        let handle = global(
            "t",
            ParameterType::thread_handle(
                false,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
        );
        refused(
            builder.emit_thread_send_runtime_helper_call(
                "thread.send",
                "_mfb_rt_thread_send",
                &[handle, constant(ParameterType::Integer, "1")],
                &ParameterType::Integer,
                false,
            ),
            "a send typed as returning an Integer",
        )
    });
    assert!(
        error.contains("expected Nothing result, got 'Integer'"),
        "{error}"
    );
}

/// A record is built from one slot per field, and a block is sized only for a type
/// whose layout is known.
#[test]
fn a_record_needs_a_slot_per_field_and_a_block_a_known_layout() {
    with_builder(|builder| {
        let slot = builder.allocate_stack_object("field", 8);
        let error = refused(
            builder.emit_build_inlined_record(&ParameterType::declared("Pt"), &[slot]),
            "a two-field record built from one slot",
        );
        assert!(error.contains("expected 2 fields, got 1"), "{error}");
        let error = refused(
            builder.emit_inlined_block_size_from_ptr_slot(
                &ParameterType::declared("Undeclared"),
                slot,
                slot,
            ),
            "the block size of an undeclared type",
        );
        assert!(
            error.contains("field size not available for type 'Undeclared'"),
            "{error}"
        );
    });
}
