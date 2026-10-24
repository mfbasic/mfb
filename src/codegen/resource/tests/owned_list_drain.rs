//! bug-843: a container's DYNAMIC owned-list (what its closures carry) is drained by
//! asking each closure's environment routine, never by the per-kind walk
//! (`cleanup/owned/builder_owned_cleanup/drain.rs`). Handed one, the per-kind walk
//! refuses by name rather than closing a closure node through a resource close op.

use crate::codegen::engine::builder::{OwnedListCleanup, OwnedListDrop, TypeModel};
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::os::linux::flavor::LinuxFlavor;
use crate::types::ParameterType;

#[test]
fn the_per_kind_walk_refuses_a_dynamic_owned_list() {
    let platform = crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    );
    let harness = BuilderHarness {
        type_model: TypeModel::empty(),
        ..BuilderHarness::default()
    };
    let mut builder = harness.builder("_mfb_owned_list_drain_probe", &platform);
    let head_slot = builder.allocate_stack_object("owned_list_head", 8);
    let reclaim_slot = builder.allocate_stack_object("owned_list_reclaim", 8);
    let cleanup = OwnedListCleanup {
        name: "bag".to_string(),
        head_slot,
        drop: OwnedListDrop::Dynamic,
        owns_elements: false,
        scope_depth: 1,
        leaf: ParameterType::func(vec![], ParameterType::Integer),
        reclaim_slot: Some(reclaim_slot),
    };
    let Err(err) = builder.emit_owned_list_reclaim(&cleanup) else {
        panic!("a dynamic list has no per-kind reclaim walk");
    };
    assert!(
        err.contains("the dynamic owned-list of 'bag' reached the per-kind drain (bug-843)"),
        "{err}"
    );
}
