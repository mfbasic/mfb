//! An in-place arm asks its destination (`collection/assign/inplace_dest.rs`) for the
//! block it may write and, for an inlined field, the field's offset or a grow through
//! the record. A field destination that has not been OPENED has no block yet, and a
//! plain local has no field: asking either is a bug in the arm — by then the container
//! has been matched — so it is refused by name, never answered with a slot that holds
//! something else. These rows drive each question with the destination it must refuse.

use crate::codegen::collection::assign::inplace_dest::InPlaceDest;
use crate::codegen::engine::builder::CodeBuilder;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{harness, model};
use crate::types::ParameterType;

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(Vec::new());
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("inplace", &platform);
    body(&mut builder)
}

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// The three unopened field destinations.
fn unopened() -> [InPlaceDest; 3] {
    [
        InPlaceDest::StateField {
            resource: "f".to_string(),
            field_index: 0,
            path: Vec::new(),
        },
        InPlaceDest::GlobalField {
            name: "g".to_string(),
            field_index: 0,
            path: Vec::new(),
        },
        InPlaceDest::RefField {
            ref_slot: 8,
            field_index: 0,
            path: Vec::new(),
        },
    ]
}

#[test]
fn an_unopened_field_destination_has_no_block_or_grow() {
    with_builder(|builder| {
        for dest in unopened() {
            let error = refused(
                builder.inplace_collection_slot(&dest),
                "the block of an unopened field",
            );
            assert!(error.contains("must be opened first"), "{error}");
            let error = refused(
                builder.inplace_inline_grow(&dest),
                "a grow through an unopened field",
            );
            assert!(error.contains("must be opened first"), "{error}");
        }
    });
}

#[test]
fn a_plain_local_destination_has_no_inlined_field() {
    with_builder(|builder| {
        let local = InPlaceDest::Direct { slot: 16 };
        let error = refused(
            builder.open_inplace_inlined_field_offset(&local),
            "the field offset of a plain local",
        );
        assert!(
            error.contains("needs an inlined-field destination"),
            "{error}"
        );
        let error = refused(
            builder.open_inplace_inlined_subblock(&local),
            "the sub-block of a plain local",
        );
        assert!(
            error.contains("needs an inlined-field destination"),
            "{error}"
        );
        let list = ParameterType::list_of(ParameterType::Integer);
        let error = refused(
            builder.lower_inplace_inlined_list_grow(
                &local,
                false,
                &list,
                &ParameterType::Integer,
                24,
            ),
            "an inlined grow of a plain local",
        );
        assert!(
            error.contains("inlined grow of List OF Integer needs an inlined-field destination"),
            "{error}"
        );
    });
}
