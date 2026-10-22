//! The data-object pre-pass's static type oracles (`memory/data/data_objects/
//! static_names.rs`): `static_type_name_with_types` and its `typeName`-fold twin.
//!
//! The pre-pass decides which strings and error messages get data objects before
//! any function is lowered, from these answers; the builder later looks those up.
//! A pre-pass that types a value differently from the builder leaves a literal with
//! no data object ("has no data object") or interns one nothing uses. These rows pin
//! the answer for each NIR value form the corpus reaches only inside larger shapes —
//! the same table `static_type_oracles.rs` pins for the builder's twin.

use crate::codegen::engine::types::FieldTypes;
use crate::codegen::memory::data::{
    static_type_name_for_fold_with_types, static_type_name_with_types,
};
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;
use std::collections::HashMap;

fn constant(type_: ParameterType) -> NirValue {
    NirValue::Const {
        type_,
        value: "0".to_string(),
    }
}

fn call_result(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::CallResult {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

#[test]
fn the_prepass_types_every_typed_value_form() {
    let types = HashMap::new();
    let fields = FieldTypes::new();
    let integer = constant(ParameterType::Integer);
    let result = constant(ParameterType::ResultOf(Box::new(ParameterType::Integer)));
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
            "a closure",
            NirValue::Closure {
                name: "f".to_string(),
                type_: ParameterType::func(Vec::new(), ParameterType::Integer),
                captures: Vec::new(),
            },
            Some(ParameterType::func(Vec::new(), ParameterType::Integer)),
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
            "a union wrap (the union)",
            NirValue::UnionWrap {
                union_type: ParameterType::declared("Shape"),
                member_type: ParameterType::declared("Circle"),
                value: Box::new(integer.clone()),
            },
            Some(ParameterType::declared("Shape")),
        ),
        (
            "a union extract (the member)",
            NirValue::UnionExtract {
                type_: ParameterType::declared("Circle"),
                value: Box::new(integer.clone()),
            },
            Some(ParameterType::declared("Circle")),
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
            "a trapped tabulated call",
            call_result("len", vec![constant(ParameterType::String)]),
            Some(ParameterType::Integer),
        ),
        (
            "a Result's Ok test",
            NirValue::ResultIsOk {
                value: Box::new(result.clone()),
            },
            Some(ParameterType::Boolean),
        ),
        (
            "a Result's value (its success type)",
            NirValue::ResultValue {
                value: Box::new(result.clone()),
            },
            Some(ParameterType::Integer),
        ),
        (
            "a non-Result read as a Result's value (its own type)",
            NirValue::ResultValue {
                value: Box::new(integer.clone()),
            },
            Some(ParameterType::Integer),
        ),
        (
            "a Result's error",
            NirValue::ResultError {
                value: Box::new(result),
            },
            Some(ParameterType::named("Error")),
        ),
        (
            "a map entry's unknown member",
            NirValue::MemberAccess {
                target: Box::new(constant(ParameterType::map_entry_of(
                    ParameterType::String,
                    ParameterType::Integer,
                ))),
                member: "size".to_string(),
            },
            None,
        ),
    ];
    for (what, value, expected) in rows {
        assert_eq!(
            static_type_name_with_types(&value, &types, &fields),
            expected,
            "{what}"
        );
    }
}

/// The fold twin types a call the table does not name through the registry resolver,
/// and a call through a local only when that local is a function.
#[test]
fn the_fold_twin_resolves_calls_the_table_leaves_open() {
    let fields = FieldTypes::new();
    let repeated = call_result(
        "strings.repeat",
        vec![
            constant(ParameterType::String),
            constant(ParameterType::Integer),
        ],
    );
    let types = HashMap::new();
    assert_eq!(
        static_type_name_with_types(&repeated, &types, &fields),
        None
    );
    assert_eq!(
        static_type_name_for_fold_with_types(&repeated, &types, &fields),
        Some(ParameterType::String)
    );
    let types = HashMap::from([("pick".to_string(), ParameterType::Integer)]);
    assert_eq!(
        static_type_name_for_fold_with_types(&call_result("pick", Vec::new()), &types, &fields),
        None,
        "a local that is not a function has no call result"
    );
}
