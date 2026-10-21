//! Native constant folding of `toString` and the `strings` case functions.
//!
//! The native backends fold a `toString` of a literal, a concatenation of
//! literals and the pure `strings` case/normalization calls at compile time. A
//! fold that is wrong prints the wrong text in every program that uses it; one
//! that folds a call it should not (a `strings.trim` it has no rule for) would
//! replace a runtime call with a constant it made up.

use crate::target::shared::nir::constfold::{
    native_constant_value, native_static_string_value, native_strings_package_static_string_value,
};
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::target::shared::runtime::RuntimeHelper;
use crate::types::ParameterType;
use std::collections::HashMap;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn runtime_to_string(arg: NirValue) -> NirValue {
    NirValue::RuntimeCall {
        helper: RuntimeHelper::General,
        target: "toString".to_string(),
        args: vec![arg],
        loc: NirSourceLoc::default(),
    }
}

fn string_of(value: Option<NirValue>) -> Option<String> {
    match value {
        Some(NirValue::Const {
            type_: ParameterType::String,
            value,
        }) => Some(value),
        _ => None,
    }
}

/// `toString` of a literal folds to a String literal whether it is a plain call
/// or a runtime call, and renders a Boolean the way the runtime does.
#[test]
fn to_string_of_a_literal_folds_for_both_call_shapes() {
    let none = HashMap::new();
    let five = constant(ParameterType::Integer, "5");
    assert_eq!(
        string_of(native_constant_value(
            &runtime_to_string(five.clone()),
            &none
        )),
        Some("5".to_string())
    );
    assert_eq!(
        native_static_string_value(&runtime_to_string(five), &none),
        Some("5".to_string())
    );
    assert_eq!(
        native_static_string_value(
            &runtime_to_string(constant(ParameterType::Boolean, "true")),
            &none
        ),
        Some("TRUE".to_string())
    );
}

/// The case and normalization calls fold; any other `strings` call does not.
#[test]
fn only_the_pure_strings_case_calls_fold() {
    let none = HashMap::new();
    let word = [constant(ParameterType::String, "Straße")];
    assert_eq!(
        native_strings_package_static_string_value("strings.upper", &word, &none),
        Some("STRASSE".to_string())
    );
    assert_eq!(
        native_strings_package_static_string_value("strings.lower", &word, &none),
        Some("straße".to_string())
    );
    assert_eq!(
        native_strings_package_static_string_value("strings.trim", &word, &none),
        None
    );
}
