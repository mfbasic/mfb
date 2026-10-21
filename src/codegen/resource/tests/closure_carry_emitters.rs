//! The closure-carry emitters' own contracts, driven directly.
//!
//! `closure_carry_emitters` states what each emitter promises about the module's
//! closure kinds and the operations it is asked for, in the cases a whole-program
//! lowering never produces because the front end and the escape analysis decide
//! first: a resource that floats to a holder of function values under a different
//! `STATE` spelling than the closure that carries it, a resource no closure in the
//! module carries at all, an operation the routine was never given. Each of those
//! is a refusal with a message naming the bug it guards, and the alternative to the
//! refusal is a node listed under the wrong kind — a record closed by the wrong
//! close op at the end of the scope.

use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::error::constants::CLOSURE_ENV_OP_RELEASE;
use crate::os::linux::flavor::LinuxFlavor;
use crate::types::ParameterType;

fn platform(
) -> crate::target::linux_common::code::Platform<crate::target::linux_x86_64::code::X86_64> {
    crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    )
}

fn model(captured: &[ParameterType]) -> TypeModel {
    let mut model = TypeModel::empty();
    model.closure_capture_types = captured.to_vec();
    model
}

fn file_with_state(state: &str) -> ParameterType {
    ParameterType::declared(&format!("fs.File STATE {state}"))
}

/// A resource listed on a dynamic owned-list takes its own kind, else the ONE kind
/// that shares its base resource under another `STATE`: a `fs::File` floating to a
/// holder whose closures carry `fs::File STATE Cur` is that kind. Two such kinds are
/// ambiguous, and so is none; both are refused rather than guessed.
#[test]
fn a_resource_is_listed_under_its_own_kind_or_the_only_one_sharing_its_base() {
    let bare = ParameterType::declared("fs.File");
    let platform = platform();

    // The only kind with the same base resource, spelled with a STATE.
    let mut harness = BuilderHarness {
        type_model: model(&[file_with_state("Cur")]),
        ..Default::default()
    };
    // The push's allocation-failure arm raises `ErrOutOfMemory`, whose message the
    // string pass would have registered.
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_str_oom".to_string());
    let mut builder = harness.builder("push", &platform);
    let (value, address, token) = (
        builder.allocate_stack_object("v", 8),
        builder.allocate_stack_object("a", 8),
        builder.allocate_stack_object("t", 8),
    );
    builder
        .emit_dynamic_owned_push(value, &bare, address, token, false)
        .expect("a bare File lists under the one File kind the closures carry");

    // No closure carries a File at all: a refusal naming the cause.
    let harness = BuilderHarness {
        type_model: model(&[ParameterType::declared("tcp.Socket")]),
        ..Default::default()
    };
    let mut builder = harness.builder("push", &platform);
    let (value, address, token) = (
        builder.allocate_stack_object("v", 8),
        builder.allocate_stack_object("a", 8),
        builder.allocate_stack_object("t", 8),
    );
    let err = builder
        .emit_dynamic_owned_push(value, &bare, address, token, false)
        .expect_err("no closure carries a File");
    assert!(
        err.contains("no closure in this module carries that resource"),
        "the refusal must say why: {err}"
    );

    // Two Files under different STATEs and a bare leaf: ambiguous, not guessed.
    let harness = BuilderHarness {
        type_model: model(&[file_with_state("A"), file_with_state("B")]),
        ..Default::default()
    };
    let mut builder = harness.builder("push", &platform);
    let (value, address, token) = (
        builder.allocate_stack_object("v", 8),
        builder.allocate_stack_object("a", 8),
        builder.allocate_stack_object("t", 8),
    );
    assert!(
        builder
            .emit_dynamic_owned_push(value, &bare, address, token, false)
            .is_err(),
        "a leaf matching two kinds by base must not pick one"
    );
}

/// An operation passes at most two argument words (the environment routine's third
/// and fourth parameters); a third is refused, not silently dropped.
#[test]
fn a_closure_resource_operation_takes_at_most_two_arguments() {
    let platform = platform();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("op", &platform);
    let slots: Vec<usize> = (0..3)
        .map(|n| builder.allocate_stack_object(&format!("arg{n}"), 8))
        .collect();
    let err = builder
        .emit_closure_resource_op(slots[0], CLOSURE_ENV_OP_RELEASE, &slots)
        .expect_err("three arguments do not fit");
    assert!(
        err.contains("takes at most two arguments, got 3"),
        "the refusal must count the arguments: {err}"
    );
}

/// An environment routine asked for an operation outside the set refuses it by number.
#[test]
fn an_environment_routine_refuses_an_unknown_operation() {
    let platform = platform();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("env", &platform);
    let env = builder.allocate_stack_object("env", 8);
    let err = builder
        .emit_closure_env_resource_op(99, env, &[], &[(0, ParameterType::declared("fs.File"))])
        .expect_err("operation 99 does not exist");
    assert!(
        err.contains("unknown closure resource operation 99"),
        "the refusal must name the operation: {err}"
    );
}

/// A listed record is closed by its leaf's close op; a type with none is refused.
#[test]
fn a_listed_record_of_a_type_with_no_close_op_is_refused() {
    let platform = platform();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("close", &platform);
    let (record, tag) = (
        builder.allocate_stack_object("record", 8),
        builder.allocate_stack_object("tag", 8),
    );
    let err = builder
        .emit_listed_record_close(
            record,
            tag,
            &ParameterType::declared("NotAResource"),
            true,
            "next",
        )
        .expect_err("a plain type has no close op");
    assert!(
        err.contains("has no registered close op"),
        "the refusal must say what is missing: {err}"
    );
}
