//! `FOR EACH` (`engine/control/builder_control/for_each.rs`) refuses an iterable it
//! has no walk for, driven on a builder directly.
//!
//! The type checker only lets a collection reach a `FOR EACH`, and only one whose
//! element, key and value types the collection layout can read. The loop still
//! checks both: a target that is not a collection is refused by name, and an
//! element the layout cannot load is refused by the loader — rather than walking
//! some other block's entries, or reading an element with the wrong width.

use crate::codegen::engine::builder::{CodeBuilder, LocalValue};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{harness, model};
use crate::target::shared::nir::{NirOp, NirValue};
use crate::types::ParameterType;

fn refused(builder: &mut CodeBuilder<'_>, iterable: NirValue, item: ParameterType) -> String {
    let op = NirOp::ForEach {
        name: "item".to_string(),
        type_: item,
        iterable,
        body: Vec::new(),
    };
    match builder.lower_ops(std::slice::from_ref(&op)) {
        Err(error) => error,
        Ok(()) => panic!("the FOR EACH must be refused"),
    }
}

/// Lower `FOR EACH item IN xs` on a builder that binds `xs` as a local of `type_`;
/// the refusal it must give.
fn over_local(type_: ParameterType, item: ParameterType) -> String {
    let model = model(Vec::new());
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("for_each", &platform);
    let stack_offset = builder.allocate_stack_object("xs", 8);
    builder.locals.insert(
        "xs".to_string(),
        LocalValue {
            type_,
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    refused(&mut builder, NirValue::Local("xs".to_string()), item)
}

fn ghost() -> ParameterType {
    ParameterType::declared("Ghost")
}

#[test]
fn a_for_each_over_a_value_that_is_not_a_collection_is_refused() {
    assert_eq!(
        over_local(ParameterType::Integer, ParameterType::Integer),
        "native code FOR EACH target 'Integer' is not a collection while lowering for each item"
    );
}

#[test]
fn a_for_each_whose_element_the_layout_cannot_load_is_refused() {
    let unsupported =
        "native collection packed payload does not support type 'Ghost' while lowering for each item";
    // A map's key, then its value; a set's element.
    assert_eq!(
        over_local(
            ParameterType::MapOf(Box::new(ghost()), Box::new(ParameterType::Integer)),
            ParameterType::named("MapEntry"),
        ),
        unsupported
    );
    assert_eq!(
        over_local(
            ParameterType::MapOf(Box::new(ParameterType::Integer), Box::new(ghost())),
            ParameterType::named("MapEntry"),
        ),
        unsupported
    );
    assert_eq!(
        over_local(ParameterType::SetOf(Box::new(ghost())), ghost()),
        unsupported
    );
}
