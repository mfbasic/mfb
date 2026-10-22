//! The three error exits a `FAIL` lowers through (`error/emission/
//! builder_error_emission.rs`) — the direct return, the route to a function-level
//! `TRAP`, and the pending error a cleanup sequence carries out — read the code,
//! message and source fields of an `Error` record. Handed a value of another type
//! they refuse by name instead of reading those fields out of whatever it is. The
//! type checker allows only an `Error` after `FAIL`, so each exit is driven
//! directly.

use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn not_an_error() -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: "7".to_string(),
    }
}

#[test]
fn every_error_exit_refuses_a_value_that_is_not_an_error() {
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("error_exit_refusals", &TestPlatform);
    assert_eq!(
        builder
            .emit_direct_error_return(&not_an_error(), 0)
            .err()
            .as_deref(),
        Some("native code fail expects Error value, got `Integer`")
    );
    let mut builder = harness.builder("error_exit_refusals", &TestPlatform);
    assert_eq!(
        builder
            .emit_direct_error_route_to_trap(&not_an_error(), 0)
            .err()
            .as_deref(),
        Some("trap routing expects Error value, got `Integer`")
    );
    let mut builder = harness.builder("error_exit_refusals", &TestPlatform);
    assert_eq!(
        builder
            .store_pending_error_from_value(&not_an_error())
            .err()
            .as_deref(),
        Some("cleanup error exit expects Error value, got `Integer`")
    );
}
