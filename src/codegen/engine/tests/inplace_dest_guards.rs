//! Two guards of the in-place destinations (`collection/assign/inplace_dest.rs`),
//! driven directly on a builder.
//!
//! bug-689: a growing route at a record field reallocates the owner's block; when the
//! owner is an ELEMENT inside a list's data region (an element update), a grow would
//! free memory inside the list. `field_is_last_inlined` keeps every growing route
//! away from such an owner; a route that reaches `inplace_inline_grow` with one
//! anyway is refused by name. And a `STATE` destination for a resource the frame has
//! no binding for is refused rather than read from a slot that is not there.

use crate::codegen::collection::assign::inplace_dest::{InPlaceDest, WriteBack};
use crate::codegen::engine::builder::LocalValue;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::types::ParameterType;

#[test]
fn a_growing_route_at_a_list_elements_block_is_refused() {
    let harness = BuilderHarness::default();
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_inplace_dest_probe", &platform);
    let block_slot = builder.allocate_stack_object("e", 8);
    builder.locals.insert(
        "e".to_string(),
        LocalValue {
            type_: ParameterType::declared("Bag"),
            stack_offset: block_slot,
            constant: None,
            by_ref: false,
        },
    );
    builder.element_updates.field_owner = Some("e".to_string());
    let dest = InPlaceDest::Inlined {
        block_slot,
        field_index: 0,
        path: Vec::new(),
        write_back: WriteBack::None,
    };
    let Err(err) = builder.inplace_inline_grow(&dest) else {
        panic!("a list element's block must not be grown");
    };
    assert!(
        err.contains("native in-place: a growing route reached list element 'e'"),
        "{err}"
    );
}

#[test]
fn a_state_destination_for_an_unbound_resource_is_refused() {
    let harness = BuilderHarness::default();
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_inplace_dest_probe", &platform);
    let Err(err) = builder.open_inplace_state_dest("ghost", 0, Vec::new()) else {
        panic!("no binding named `ghost`");
    };
    assert!(
        err.contains("native code state assignment unknown local 'ghost'"),
        "{err}"
    );
}
