//! A native collection lowering refuses an argument shape the type checker would have
//! stopped, naming what was wrong.
//!
//! `partition`, `findLastIndex` and `toSet` lower from NIR values (the call site's own
//! monomorph target names the element type), `filter` and `insert` from lowered
//! arguments. Each checks what it was handed before it emits a byte: the list's element
//! is the one the target names, the predicate is a function, a function that is a
//! predicate returns `Boolean`, an inserted item is one item and not a list. A program
//! cannot hand them anything else, so each refusal is dead to the corpus; the
//! alternative to it is a lowering that reads a list at the stride of another type, or
//! calls through a register that holds an `Integer`.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, Body};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn list(element: ParameterType) -> NirValue {
    NirValue::ListLiteral {
        type_: ParameterType::list_of(element),
        values: Vec::new(),
    }
}

fn number() -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: "0".to_string(),
    }
}

fn collections_member(member: &str) -> &'static crate::codegen::registry::RegistryFunction {
    registry()
        .packages()
        .iter()
        .find(|package| package.import_name() == "collections")
        .expect("the collections package")
        .function(member)
        .expect("a collections member")
}

/// The registry's fast path for `collections::<member>`.
fn fast_path(member: &str) -> crate::codegen::registry::MfbFastPath {
    collections_member(member)
        .implementations()
        .iter()
        .find_map(|implementation| match implementation.body {
            Body::Mfb {
                fast_path: Some(fast),
                ..
            } => Some(fast),
            _ => None,
        })
        .unwrap_or_else(|| panic!("collections::{member} has a native fast path"))
}

/// The error the member's fast path gives for `target` and `args`, which must refuse.
fn refusal(member: &str, target: &str, args: &[NirValue]) -> String {
    let fast = fast_path(member);
    let platform = TestPlatform;
    let mut harness = BuilderHarness::default();
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_str_oom".to_string());
    let mut builder = harness.builder("_mfb_probe", &platform);
    match fast(&mut builder, target, args) {
        Err(err) => err,
        Ok(_) => panic!("{target} must refuse these arguments"),
    }
}

#[test]
fn partition_refuses_a_mismatched_list_and_a_non_function_predicate() {
    let err = refusal(
        "partition",
        "#collections_partition$Integer",
        &[list(ParameterType::Float), number()],
    );
    assert!(err.contains("element mismatch"), "{err}");
    let err = refusal(
        "partition",
        "#collections_partition$Integer",
        &[list(ParameterType::Integer), number()],
    );
    assert!(err.contains("predicate must be a function"), "{err}");
}

#[test]
fn find_last_index_refuses_a_non_list_and_a_non_function_predicate() {
    let err = refusal(
        "findLastIndex",
        "#collections_findLastIndex$String",
        &[number(), number(), number()],
    );
    assert!(err.contains("does not accept"), "{err}");
    let err = refusal(
        "findLastIndex",
        "#collections_findLastIndex$String",
        &[list(ParameterType::String), number(), number()],
    );
    assert!(err.contains("predicate must be a function"), "{err}");
}

#[test]
fn to_set_refuses_a_list_of_another_element_type() {
    let err = refusal(
        "toSet",
        "#collections_toSet$Integer",
        &[list(ParameterType::Float)],
    );
    assert!(err.contains("element mismatch"), "{err}");
}

/// Lower the registry's `collections::<member>` body (the overload with `types.len()`
/// parameters) over `types`, and return its refusal.
fn member_refusal(member: &str, types: Vec<ParameterType>) -> String {
    let platform = TestPlatform;
    let implementation = collections_member(member)
        .implementations()
        .iter()
        .find(|implementation| implementation.params.len() == types.len())
        .expect("an overload of that arity");
    let Body::AbiInline(lower) = implementation.body else {
        panic!("collections::{member} lowers through abi_inline");
    };
    let mut vregs = Vregs::new();
    let args: Vec<ValueResult> = types
        .into_iter()
        .map(|type_| ValueResult {
            type_,
            location: Operand::from(vregs.next()),
            text: "arg".to_string(),
            origin: None,
        })
        .collect();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("_mfb_probe", &platform);
    let ctx = harness.abi_ctx(&platform);
    match lower(&mut builder, &args, &ctx) {
        Err(err) => err,
        Ok(_) => panic!("collections::{member} must refuse these arguments"),
    }
}

#[test]
fn filter_refuses_a_predicate_that_does_not_return_boolean() {
    let err = member_refusal(
        "filter",
        vec![
            ParameterType::list_of(ParameterType::Integer),
            ParameterType::func(vec![ParameterType::Integer], ParameterType::Integer),
        ],
    );
    assert!(err.contains("predicate must return Boolean"), "{err}");
}

#[test]
fn insert_refuses_a_list_where_one_item_belongs() {
    let numbers = ParameterType::list_of(ParameterType::Integer);
    let err = member_refusal(
        "insert",
        vec![numbers.clone(), ParameterType::Integer, numbers],
    );
    assert!(err.contains("single item, not a list"), "{err}");
}
