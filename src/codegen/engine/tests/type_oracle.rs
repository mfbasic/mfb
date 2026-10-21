//! The static type oracle over NIR values (`engine/types/type_utils.rs`): what a
//! module-level walk concludes a value's type is without a builder. Each row is a
//! node shape and the type it must answer, so a predicate built on this seam
//! (the Float-error-message pre-pass, the enum `toString` data) sees a node as the
//! lowering that follows will.

use crate::codegen::engine::types::{
    collection_type_code, local_constant_value_with_constants, static_nir_value_type,
    to_string_enum_members, FieldTypes,
};
use crate::codegen::error::constants::COLLECTION_TYPE_NOTHING;
use crate::target::shared::nir::{NirRecordUpdate, NirSourceLoc, NirValue};
use crate::target::shared::runtime::RuntimeHelper;
use crate::types::ParameterType;
use std::collections::HashMap;

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn locals(pairs: &[(&str, ParameterType)]) -> HashMap<String, ParameterType> {
    pairs
        .iter()
        .map(|(name, type_)| (name.to_string(), type_.clone()))
        .collect()
}

/// Nodes that carry their type, nodes that derive it, and nodes that cannot be
/// typed statically.
#[test]
fn a_node_is_typed_by_what_it_carries_or_derives() {
    let fields = FieldTypes::new();
    let scope = locals(&[
        ("n", ParameterType::Integer),
        ("r", ParameterType::result_of(ParameterType::Float)),
    ]);
    let list = ParameterType::list_of(ParameterType::Integer);
    let type_of = |value: &NirValue| static_nir_value_type(value, &scope, &fields);

    // Carried: the node's own annotation.
    for (value, expected) in [
        (
            NirValue::LocalRef {
                name: "n".to_string(),
                type_: ParameterType::Integer,
            },
            ParameterType::Integer,
        ),
        (
            NirValue::Capture {
                index: 0,
                type_: ParameterType::String,
                by_ref: false,
            },
            ParameterType::String,
        ),
        (
            NirValue::UnionExtract {
                type_: ParameterType::Float,
                value: Box::new(local("n")),
            },
            ParameterType::Float,
        ),
        (
            NirValue::WithUpdate {
                type_: ParameterType::named("Rec"),
                target: Box::new(local("n")),
                updates: Vec::<NirRecordUpdate>::new(),
            },
            ParameterType::named("Rec"),
        ),
        (
            NirValue::Checked {
                type_: ParameterType::Float,
                value: Box::new(local("n")),
                observe: true,
            },
            ParameterType::Float,
        ),
        (
            NirValue::ListLiteral {
                type_: list.clone(),
                values: Vec::new(),
            },
            list.clone(),
        ),
    ] {
        assert_eq!(type_of(&value), Some(expected));
    }

    // Derived: a unary keeps its operand's type, a result test is a Boolean, a
    // result's value is its success type and its error is an `Error`.
    assert_eq!(
        type_of(&NirValue::Unary {
            op: crate::operators::UnaryOp::Negate,
            operand: Box::new(local("n")),
            loc: NirSourceLoc::default(),
        }),
        Some(ParameterType::Integer)
    );
    assert_eq!(
        type_of(&NirValue::ResultIsOk {
            value: Box::new(local("r")),
        }),
        Some(ParameterType::Boolean)
    );
    assert_eq!(
        type_of(&NirValue::ResultValue {
            value: Box::new(local("r")),
        }),
        Some(ParameterType::Float)
    );
    assert_eq!(
        type_of(&NirValue::ResultValue {
            value: Box::new(local("n")),
        }),
        None,
        "the value of something that is not a result has no static type"
    );
    assert_eq!(
        type_of(&NirValue::ResultError {
            value: Box::new(local("r")),
        }),
        Some(ParameterType::named("Error"))
    );
    // A call through a function-valued local types as its `FUNC`'s return.
    let callable = locals(&[(
        "g",
        ParameterType::Func(
            vec![ParameterType::Float],
            Box::new(ParameterType::Float),
            false,
        ),
    )]);
    assert_eq!(
        static_nir_value_type(
            &call("g", vec![constant(ParameterType::Float, "1.0")]),
            &callable,
            &fields
        ),
        Some(ParameterType::Float)
    );
    // `RuntimeCall` types as the same call's `Call` does.
    let runtime = NirValue::RuntimeCall {
        helper: RuntimeHelper::General,
        target: "toString".to_string(),
        args: vec![constant(ParameterType::Integer, "1")],
        loc: NirSourceLoc::default(),
    };
    assert_eq!(
        type_of(&runtime),
        type_of(&call(
            "toString",
            vec![constant(ParameterType::Integer, "1")]
        ))
    );
    // Not statically typed.
    assert_eq!(type_of(&local("unknown")), None);
}

/// `h.state` is the resource handle's payload record, `p.result` of a thread
/// handle is the `Result` of its output, and a map entry has `key` and `value`.
#[test]
fn a_member_read_is_typed_as_a_state_a_thread_result_or_a_map_entry() {
    let mut fields = FieldTypes::new();
    fields.insert(
        ("Rec".to_string(), "count".to_string()),
        ParameterType::Integer,
    );
    let entry_type = ParameterType::map_entry_of(ParameterType::String, ParameterType::Float);
    let scope = locals(&[("rec", ParameterType::named("Rec"))]);
    let member = |target: NirValue, name: &str| NirValue::MemberAccess {
        target: Box::new(target),
        member: name.to_string(),
    };
    // A record field, from the table.
    assert_eq!(
        static_nir_value_type(&member(local("rec"), "count"), &scope, &fields),
        Some(ParameterType::Integer)
    );
    // A member nothing declares.
    assert_eq!(
        static_nir_value_type(&member(local("rec"), "nope"), &scope, &fields),
        None
    );
    // Map-entry members, off an entry-typed local.
    let scope = locals(&[("e", entry_type)]);
    assert_eq!(
        static_nir_value_type(&member(local("e"), "key"), &scope, &fields),
        Some(ParameterType::String)
    );
    assert_eq!(
        static_nir_value_type(&member(local("e"), "value"), &scope, &fields),
        Some(ParameterType::Float)
    );
    assert_eq!(
        static_nir_value_type(&member(local("e"), "other"), &scope, &fields),
        None
    );
}

/// `Nothing` is a collection payload type with its own code, distinct from the
/// object code every unlisted type falls to.
#[test]
fn nothing_has_its_own_collection_payload_code() {
    assert_eq!(
        collection_type_code(&ParameterType::Nothing),
        Some(COLLECTION_TYPE_NOTHING)
    );
}

/// `toString(<enum member>)` needs the member names as string data: an
/// `Enum.Member` literal names its enum directly, a typed local names it by its
/// type, and anything else needs none.
#[test]
fn an_enum_to_string_argument_names_its_members() {
    let color = ParameterType::named("Color");
    let mut fields = FieldTypes::new();
    fields.insert_enum(
        color.clone(),
        vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
    );
    let scope = locals(&[("c", color), ("n", ParameterType::Integer)]);
    let literal = NirValue::MemberAccess {
        target: Box::new(local("Color")),
        member: "Green".to_string(),
    };
    let members =
        to_string_enum_members(&literal, &scope, &fields).expect("a literal names its enum");
    assert_eq!(members, ["Red", "Green", "Blue"]);
    assert!(to_string_enum_members(&local("c"), &scope, &fields).is_some());
    assert!(to_string_enum_members(&local("n"), &scope, &fields).is_none());
    // A member the enum does not declare is no literal; the type walk then finds
    // nothing for a non-enum record.
    let unknown = NirValue::MemberAccess {
        target: Box::new(local("Color")),
        member: "Mauve".to_string(),
    };
    assert!(to_string_enum_members(&unknown, &scope, &fields).is_none());
}

/// A `toString` of a constant folds to a string constant whichever form the call
/// took (`Call` or `RuntimeCall`); a local with no known constant does not.
#[test]
fn a_constant_to_string_folds_in_either_call_form() {
    let fields = FieldTypes::new();
    let scope = HashMap::new();
    let constants: HashMap<String, NirValue> =
        [("k".to_string(), constant(ParameterType::Integer, "42"))]
            .into_iter()
            .collect();
    let runtime = NirValue::RuntimeCall {
        helper: RuntimeHelper::General,
        target: "toString".to_string(),
        args: vec![local("k")],
        loc: NirSourceLoc::default(),
    };
    for value in [call("toString", vec![local("k")]), runtime] {
        match local_constant_value_with_constants(&value, &constants, &scope, &fields) {
            Some(NirValue::Const { type_, value }) => {
                assert_eq!(type_, ParameterType::String);
                assert_eq!(value, "42");
            }
            _ => panic!("a constant toString folds"),
        }
    }
    let unknown = call("toString", vec![local("free")]);
    assert!(local_constant_value_with_constants(&unknown, &constants, &scope, &fields).is_none());
}
