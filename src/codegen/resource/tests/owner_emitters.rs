//! The owner-token and module-list emitters' own contracts, driven directly.
//!
//! `resource/owner.rs` mints a scope token per nesting depth and decides, per
//! resource, whether a scope still owns it; `resource/module_owner.rs` lists a
//! resource stored into a module-level binding under the kind its bindings reach.
//! The programs that exercise them are `tests/rt-behavior/resources/**`; what no
//! program produces is the REFUSAL each emitter owes when it is handed something the
//! front end rules out — a nesting deeper than the token can encode, a resource no
//! module-level binding can reach — and the no-op it owes a value that is not a
//! resource at all. Each refusal names the bug that introduced it.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::{GlobalValue, TypeModel};
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::resource::owner::SCOPE_TOKEN_MAX_DEPTH;
use crate::os::linux::flavor::LinuxFlavor;
use crate::types::ParameterType;

type Linux = crate::target::linux_common::code::Platform<crate::target::linux_x86_64::code::X86_64>;

fn platform() -> Linux {
    crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    )
}

fn global(type_: &str, offset: usize) -> GlobalValue {
    GlobalValue {
        type_: ParameterType::declared(type_),
        offset,
    }
}

/// A scope token encodes its depth in `SCOPE_TOKEN_DEPTH_BITS` bits: depth 0 (no
/// scope) and any depth past the maximum are refused, because a token whose depth
/// wrapped would compare as an outer scope and let an inner scope's record outlive it.
#[test]
fn a_scope_depth_outside_the_token_range_is_refused() {
    let platform = platform();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("token", &platform);
    for depth in [0, SCOPE_TOKEN_MAX_DEPTH + 1] {
        let err = builder
            .emit_scope_token(depth)
            .expect_err("a depth outside 1..=max has no token");
        assert!(
            err.contains("outside the scope-token range"),
            "depth {depth}: the refusal must say why: {err}"
        );
    }
    builder
        .emit_scope_token(SCOPE_TOKEN_MAX_DEPTH)
        .expect("the deepest encodable scope has a token");
}

/// Discarding a value that is not a resource closes nothing: no call is emitted.
#[test]
fn discarding_a_non_resource_closes_nothing() {
    let platform = platform();
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("discard", &platform);
    let slot = builder.allocate_stack_object("value", 8);
    builder
        .emit_close_unowned_at(slot, &ParameterType::Integer)
        .expect("a non-resource is skipped");
    assert!(
        builder
            .instructions
            .iter()
            .all(|i| i.op != CodeOp::BranchLink && i.op != CodeOp::BranchLinkRegister),
        "a value with no close op must not be passed to one"
    );
}

/// A closure temporary at a statement is closed through its environment routine —
/// an indirect call — not by a close op of its own: the closure is not a resource,
/// what it carries is.
#[test]
fn a_closure_temporary_is_closed_through_its_environment_routine() {
    let platform = platform();
    let mut model = TypeModel::empty();
    model.closure_capture_types = vec![ParameterType::declared("fs.File")];
    let harness = BuilderHarness {
        type_model: model,
        ..Default::default()
    };
    let mut builder = harness.builder("temp", &platform);
    let slot = builder.allocate_stack_object("closure", 8);
    let closure = ParameterType::func(vec![], ParameterType::Integer);
    builder
        .emit_resource_temp_close(slot, &closure, Some(1))
        .expect("a closure temporary lowers");
    assert!(
        builder
            .instructions
            .iter()
            .any(|i| i.op == CodeOp::BranchLinkRegister),
        "the carried resources are closed by the closure's own routine, called through \
         the address in its descriptor"
    );
}

/// A resource stored into a module-level binding is listed under its own kind, else
/// the ONE kind sharing its base under another `STATE`; with none, the store is
/// refused naming bug-806 rather than listing the record under a wrong close op.
#[test]
fn a_module_stored_resource_needs_a_binding_that_reaches_it() {
    let platform = platform();
    let bare = ParameterType::declared("fs.File");

    // The only reachable File kind is spelled with a STATE: that is the kind.
    let mut harness = BuilderHarness::default();
    harness
        .globals
        .insert("g".to_string(), global("List OF fs.File STATE Cur", 0));
    harness.globals.insert(
        crate::codegen::resource::module_owner::MODULE_RESOURCES_GLOBAL.to_string(),
        global("Integer", 8),
    );
    // The list push's allocation-failure arm raises `ErrOutOfMemory`.
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_str_oom".to_string());
    let mut builder = harness.builder("float", &platform);
    let slot = builder.allocate_stack_object("value", 8);
    builder
        .emit_module_float(slot, &bare)
        .expect("a bare File lists under the one File kind a global reaches");

    // No global reaches a File at all.
    let mut harness = BuilderHarness::default();
    harness
        .globals
        .insert("g".to_string(), global("List OF tcp.Socket", 0));
    harness.globals.insert(
        crate::codegen::resource::module_owner::MODULE_RESOURCES_GLOBAL.to_string(),
        global("Integer", 8),
    );
    let mut builder = harness.builder("float", &platform);
    let slot = builder.allocate_stack_object("value", 8);
    let err = builder
        .emit_module_float(slot, &bare)
        .expect_err("no module-level binding reaches a File");
    assert!(
        err.contains("no module-level binding's type reaches that resource (bug-806)"),
        "the refusal must say why: {err}"
    );
}

/// A store into a resource-carrying global raises the resources its NAMED operands
/// hold. A store with no value writes the type's default and raises nothing, and a
/// captured operand of a type that holds no resource is not walked: in both cases
/// the store is handled (`false`: nothing left to the caller) without a call to
/// the module list.
#[test]
fn a_store_that_names_no_resource_raises_nothing() {
    let platform = platform();
    let mut harness = BuilderHarness::default();
    harness
        .globals
        .insert("g".to_string(), global("List OF fs.File", 0));
    harness.globals.insert(
        crate::codegen::resource::module_owner::MODULE_RESOURCES_GLOBAL.to_string(),
        global("Integer", 8),
    );
    let list = ParameterType::declared("List OF fs.File");
    let mut builder = harness.builder("store", &platform);
    assert_eq!(builder.emit_module_store_float("g", &list, None), Ok(false));
    let counter = crate::target::shared::nir::NirValue::Capture {
        index: 0,
        type_: ParameterType::Integer,
        by_ref: false,
    };
    assert_eq!(
        builder.emit_module_store_float("g", &list, Some(&counter)),
        Ok(false)
    );
    assert!(
        builder
            .instructions
            .iter()
            .all(|i| i.op != CodeOp::BranchLink && i.op != CodeOp::BranchLinkRegister),
        "nothing was raised, so no list push was called"
    );
}
