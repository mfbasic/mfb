//! The native collection lowerings that replace an `.mfb` body (`chunks`, `toSet`)
//! take only the shapes they were written for. Outside them the fast path DECLINES
//! — the call goes to the package body, which raises the documented error — and the
//! lowering itself refuses a source it has no layout for, by name. The front end
//! keeps the second kind out of every real program, so both are driven directly.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn integer(value: &str) -> NirValue {
    constant(ParameterType::Integer, value)
}

/// What `lower` yields on a fresh builder whose function table knows `chunks`'
/// `Integer` instantiation by its symbol, and the calls it relocates against.
fn lowered(
    lower: impl FnOnce(
        &mut crate::codegen::engine::builder::CodeBuilder<'_>,
    ) -> Result<ValueResult, String>,
) -> (Result<ValueResult, String>, Vec<String>) {
    let mut harness = BuilderHarness::default();
    harness.function_symbols.insert(
        "#collections_chunks$Integer".to_string(),
        "_mfb_ifn_collections_chunks_Integer".to_string(),
    );
    let mut builder = harness.builder("collection_native_refusals", &TestPlatform);
    let result = lower(&mut builder);
    let calls = builder
        .relocations
        .iter()
        .map(|relocation| relocation.to.clone())
        .collect();
    (result, calls)
}

/// `chunks`' native form needs its size as an `Integer` constant of at least 1.
/// A zero, a non-`Integer` constant and an unparsable one all decline, so the call
/// reaches the package body — the one place `ErrInvalidArgument` for a bad size is
/// raised.
#[test]
fn chunks_declines_a_size_it_cannot_prove_positive() {
    let list = || constant(ParameterType::list_of(ParameterType::Integer), "[]");
    for (index, size) in [
        integer("0"),
        integer("-3"),
        integer("many"),
        constant(ParameterType::Float, "2.0"),
    ]
    .into_iter()
    .enumerate()
    {
        let call = NirValue::Call {
            target: "#collections_chunks$Integer".to_string(),
            args: vec![list(), size],
            loc: NirSourceLoc::default(),
        };
        let (_, calls) = lowered(|b| b.lower_value(&call));
        assert!(
            calls
                .iter()
                .any(|to| to == "_mfb_ifn_collections_chunks_Integer"),
            "size #{index}: expected the package body to be called, got {calls:?}"
        );
    }
    // The control: a constant size of 2 takes the native form, which calls no body.
    let call = NirValue::Call {
        target: "#collections_chunks$Integer".to_string(),
        args: vec![list(), integer("2")],
        loc: NirSourceLoc::default(),
    };
    let (_, calls) = lowered(|b| b.lower_value(&call));
    assert!(
        !calls
            .iter()
            .any(|to| to == "_mfb_ifn_collections_chunks_Integer"),
        "a provable size must lower natively, got {calls:?}"
    );
}

#[test]
fn the_native_chunks_lowering_refuses_a_source_that_is_not_a_list() {
    let (result, _) = lowered(|b| b.lower_collection_chunks_call(&[integer("4")], 2));
    assert_eq!(
        result.err().as_deref(),
        Some("native chunks does not accept Integer")
    );
}

/// `toSet`'s native form presizes a fixed-width set; any other element type is
/// refused by name rather than laid out at a width it does not have.
#[test]
fn the_native_to_set_lowering_refuses_an_element_that_is_not_fixed_width() {
    let strings = constant(ParameterType::list_of(ParameterType::String), "[]");
    let (result, _) =
        lowered(|b| b.lower_collection_to_set_call(&[strings], &ParameterType::String));
    assert_eq!(
        result.err().as_deref(),
        Some("native toSet: 'String' is not fixed-width")
    );
}
