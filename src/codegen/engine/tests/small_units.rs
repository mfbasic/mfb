//! Small pure helpers of the engine whose contract is a function of their input
//! alone: the re-evaluable-place predicate, instruction field validation and the
//! closer-symbol resolution.

use crate::codegen::engine::builder::resolve_closer_symbol;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::value::reeval::is_reevaluation_safe;
use crate::target::shared::abi;
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;
use std::collections::HashMap;

/// A by-reference capture read (`LocalRef`) is a place read like a `Local`:
/// cheap and effect-free to evaluate once per lane, and so is a field read
/// chained off it; a call is not.
#[test]
fn a_by_ref_capture_read_is_reevaluable_and_a_call_is_not() {
    let by_ref = NirValue::LocalRef {
        name: "acc".to_string(),
        type_: ParameterType::Integer,
    };
    assert!(is_reevaluation_safe(&by_ref));
    let field = NirValue::MemberAccess {
        target: Box::new(by_ref),
        member: "count".to_string(),
    };
    assert!(is_reevaluation_safe(&field));
    let call = NirValue::Call {
        target: "next".to_string(),
        args: Vec::new(),
        loc: NirSourceLoc::default(),
    };
    assert!(!is_reevaluation_safe(&call));
    assert!(!is_reevaluation_safe(&NirValue::MemberAccess {
        target: Box::new(call),
        member: "count".to_string(),
    }));
}

/// The explicit-borrow subtract names five operands (two results, three
/// inputs); an instruction missing one is refused by name before any backend
/// sees it.
#[test]
fn a_borrowing_subtract_missing_an_operand_is_refused_by_name() {
    let complete = CodeInstruction::new("sub_borrow")
        .field("dst", abi::c_arg(0))
        .field("borrow_out", abi::c_arg(1))
        .field("lhs", abi::c_arg(2))
        .field("rhs", abi::c_arg(3))
        .field("borrow_in", abi::c_arg(4));
    assert_eq!(complete.validate(), Ok(()));
    let missing = CodeInstruction::new("sub_borrow")
        .field("dst", abi::c_arg(0))
        .field("borrow_out", abi::c_arg(1))
        .field("lhs", abi::c_arg(2))
        .field("rhs", abi::c_arg(3));
    let error = missing.validate().unwrap_err();
    assert!(
        error.contains("sub_borrow") && error.contains("borrow_in"),
        "{error}"
    );
}

/// `RESOURCE T CLOSE BY link::op` reaches the resolver as either the symbol's own
/// name or an identity-prefixed spelling (a 16-hex-digit content hash and a dot);
/// only that exact shape is stripped, so a dotted package name is never mistaken
/// for an identity.
#[test]
fn a_closer_resolves_by_name_or_by_identity_prefixed_name_only() {
    let symbols: HashMap<String, String> = [
        ("pkg.close".to_string(), "_pkg_close".to_string()),
        ("a.b".to_string(), "_a_b".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        resolve_closer_symbol("pkg.close", &symbols).as_deref(),
        Some("_pkg_close")
    );
    assert_eq!(
        resolve_closer_symbol("0123456789abcdef.pkg.close", &symbols).as_deref(),
        Some("_pkg_close")
    );
    // A short first segment is a package name, not an identity.
    assert_eq!(resolve_closer_symbol("xx.a.b", &symbols), None);
    // Sixteen characters that are not all hex digits are not an identity.
    assert_eq!(
        resolve_closer_symbol("0123456789abcdeg.pkg.close", &symbols),
        None
    );
    // No dot at all and not a known name.
    assert_eq!(resolve_closer_symbol("close", &symbols), None);
}

/// A callee-saved register the lowering touches is recorded once for the frame
/// finalizer to save and restore; a scratch register is never recorded.
#[test]
fn a_touched_callee_saved_register_is_recorded_once() {
    use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("units_callee_saved", &TestPlatform);
    assert!(abi::is_callee_saved("x19"));
    builder.mark_register_used("x19");
    builder.mark_register_used("x19");
    builder.mark_register_used("x0");
    assert_eq!(builder.used_callee_saved, vec!["x19".to_string()]);
}

/// What a platform that implements none of the optional emitters answers: the
/// Windows-only release of a marshalled result is refused by name, the app window
/// title falls back to the project name, and there are no app window data objects.
#[test]
fn a_platform_without_the_optional_emitters_answers_the_documented_defaults() {
    use crate::codegen::engine::tests::test_support::TestPlatform;
    use crate::codegen::engine::types::CodegenPlatform;
    let platform = TestPlatform;
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    let error = platform
        .emit_release_marshalled(
            "units",
            &HashMap::new(),
            &mut instructions,
            &mut relocations,
        )
        .unwrap_err();
    assert!(
        error.contains("only implemented on windows-x86_64"),
        "{error}"
    );
    assert!(instructions.is_empty() && relocations.is_empty());
    assert_eq!(platform.app_default_window_title("demo"), "demo");
    assert!(platform.app_window_data_objects().is_empty());
}

/// bug-567: a control transfer classified as freeing (or lowering) its temporaries
/// before it branches must leave none registered; one that did is reported as a
/// codegen error naming the leak rather than silently truncated into one. A
/// fall-through statement just clears what it registered.
#[test]
fn a_control_transfer_that_left_a_temporary_unfreed_is_reported() {
    use crate::codegen::engine::builder::PendingTemp;
    use crate::codegen::engine::control::TransferTemps;
    use crate::codegen::engine::operand::Operand;
    use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("units_transfer_temps", &TestPlatform);
    let temp = || PendingTemp {
        type_: ParameterType::String,
        slot: 0,
        location: Operand::from("x0"),
        shallow: false,
        resource: false,
        union_box: false,
    };
    for disposition in [
        TransferTemps::FreedBeforeBranch,
        TransferTemps::NoExpression,
    ] {
        builder.pending_temp_frees.push(temp());
        let error = builder
            .forget_pending_temps_to(0, disposition)
            .expect_err("a temporary above the watermark is a leak");
        assert!(error.contains("1 pending temporaries unfreed"), "{error}");
        assert!(error.contains("bug-567"), "{error}");
        builder.pending_temp_frees.clear();
    }
    builder.pending_temp_frees.push(temp());
    builder
        .forget_pending_temps_to(0, TransferTemps::StatementScope)
        .expect("a fall-through statement clears its temporaries");
    assert!(builder.pending_temp_frees.is_empty());
}
