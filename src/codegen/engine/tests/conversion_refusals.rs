//! The conversion and predicate builders (`engine/convert/builder_conversions/**`)
//! refuse an argument of a type they have no lowering for, by name, instead of
//! emitting code that reads the wrong representation. The front end never lets such
//! a call through, so each is driven directly over a constant of the wrong type:
//! the message is what a developer reads when an upstream pass hands codegen NIR it
//! should not have.

use crate::codegen::engine::builder::CodeBuilder;
use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn boolean() -> NirValue {
    constant(ParameterType::Boolean, "true")
}

fn text() -> NirValue {
    constant(ParameterType::String, "abc")
}

/// The refusal `lower` produces on a fresh builder.
fn refusal(lower: impl FnOnce(&mut CodeBuilder<'_>) -> Result<ValueResult, String>) -> String {
    let mut harness = BuilderHarness::default();
    // The string literal `text()` builds is read-only data the plan would have
    // registered.
    harness
        .string_symbols
        .insert("abc".to_string(), "_mfb_str_abc".to_string());
    let mut builder = harness.builder("conversion_refusals", &TestPlatform);
    match lower(&mut builder) {
        Ok(value) => panic!("expected a refusal, lowered to {}", value.text),
        Err(error) => error,
    }
}

#[test]
fn a_conversion_names_the_argument_type_it_cannot_take() {
    let arg = boolean();
    assert_eq!(
        refusal(|b| b.lower_to_float(&arg)),
        "native toFloat does not accept argument type 'Boolean'"
    );
    assert_eq!(
        refusal(|b| b.lower_to_fixed(&arg)),
        "native toFixed does not accept argument type 'Boolean'"
    );
    assert_eq!(
        refusal(|b| b.lower_to_money(&arg)),
        "native toMoney does not accept argument type 'Boolean'"
    );
    assert_eq!(
        refusal(|b| b.lower_to_byte(&arg)),
        "native toByte does not accept argument type 'Boolean'"
    );
    assert_eq!(
        refusal(|b| b.lower_to_scalar(&arg)),
        "native toScalar does not accept argument type 'Boolean'"
    );
}

#[test]
fn a_predicate_names_the_argument_type_it_cannot_take() {
    let integer = constant(ParameterType::Integer, "4");
    assert_eq!(
        refusal(|b| b.lower_is_numeric(&integer)),
        "native isNumeric does not accept argument type 'Integer'"
    );
    assert_eq!(
        refusal(|b| b.lower_integer_parity_predicate("isEven", &text(), false)),
        "native isEven does not accept argument type 'String'"
    );
    assert_eq!(
        refusal(|b| b.lower_numeric_filter_predicate("isPositive", &text())),
        "native isPositive does not accept argument type 'String'"
    );
}

/// The `isPositive` / `isNegative` / `isZero` and `isEmpty` / `isNotEmpty` pairs
/// are the only names the filter builders implement; any other is a refusal that
/// names it.
#[test]
fn a_filter_predicate_the_builder_does_not_implement_is_named() {
    let integer = constant(ParameterType::Integer, "4");
    assert_eq!(
        refusal(|b| b.lower_numeric_filter_predicate("isBogus", &integer)),
        "native filter predicate 'isBogus' is not implemented"
    );
    assert_eq!(
        refusal(|b| b.lower_empty_filter_predicate("isBogus", &text())),
        "native filter predicate 'isBogus' is not implemented"
    );
}
