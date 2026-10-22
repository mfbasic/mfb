//! The owned-list bookkeeping (`cleanup/owned/builder_owned_cleanup/`) refuses a list
//! or a float it has no table entry for, rather than pushing a record onto no list —
//! where nothing would close it — and floats nothing for a store it never recorded.
//!
//! The owned-list a container gets is decided from its type (§15.6), and the floats
//! into it from the same type tables, so a program never asks for a list that is not
//! there. These rows drive the bookkeeping directly, on a hand-built type model, with
//! the requests the type rules rule out.

use crate::codegen::engine::builder::CodeBuilder;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{harness, model, record};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// A model with a declared `RESOURCE Db`.
fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let mut db = record("Db", Vec::new());
    db.kind = "resource".to_string();
    let model = model(vec![db]);
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("owned", &platform);
    body(&mut builder)
}

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// A container type that reaches no resource (and holds no function value) gets no
/// owned-list.
#[test]
fn a_type_that_reaches_no_resource_has_no_owned_list() {
    let error = with_builder(|builder| {
        refused(
            builder.setup_owned_list("xs", &ParameterType::list_of(ParameterType::Integer)),
            "an owned-list for a List OF Integer",
        )
    });
    assert!(
        error.contains("owner 'xs' of type 'List OF Integer' reaches no resource"),
        "{error}"
    );
}

/// A resource floated into a container that has no owned-list is refused.
#[test]
fn a_float_into_a_container_without_a_list_is_refused() {
    let error = with_builder(|builder| {
        let slot = builder.allocate_stack_object("resource", 8);
        refused(
            builder.emit_owned_list_push("nothing", slot, &ParameterType::declared("Db")),
            "a resource floated to a container with no list",
        )
    });
    assert!(
        error.contains("floats to 'nothing', which has no owned-list for it"),
        "{error}"
    );
}

/// The float that follows a container assignment or a store through a captured
/// reference is emitted only for a store the analysis recorded, into a binding that
/// exists; anything else floats nothing.
#[test]
fn a_float_with_nothing_recorded_emits_nothing() {
    with_builder(|builder| {
        let before = builder.instructions.len();
        let value = NirValue::Local("hs".to_string());
        builder
            .emit_assigned_container_float("hs", &value)
            .expect("an unrecorded store is no error");
        let slot = builder.allocate_stack_object("value", 8);
        builder
            .emit_captured_store_float("hs", slot, &ParameterType::declared("Db"))
            .expect("a store into no captured binding is no error");
        assert_eq!(builder.instructions.len(), before, "nothing is floated");
    });
}
