//! A field update's owner block (`engine/control/builder_control/field_replace.rs`):
//! loading it and installing a replacement, driven on a builder directly.
//!
//! The in-place field arms reach these with an owner their own admission already
//! resolved — a bound record local, a bound `RES` handle, a global — and never
//! install over a `STATE` payload (`replace_field_admitted` admits none). Each still
//! refuses by name when the premise fails, rather than loading or overwriting the
//! slot of a local nobody bound, or repointing a resource's shared `STATE` behind its
//! aliases' backs.

use crate::codegen::collection::assign::self_update::FieldContainer;
use crate::codegen::engine::builder::{CodeBuilder, LocalValue};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record};
use crate::types::ParameterType;

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(vec![record("Pt", vec![field("x", "Integer")])]);
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("field_owner", &platform);
    body(&mut builder)
}

#[test]
fn an_owner_that_is_not_bound_is_refused() {
    with_builder(|builder| {
        assert_eq!(
            refused(
                builder.emit_field_owner_block(FieldContainer::Record { local: "ghost" }),
                "loading an unbound record local"
            ),
            "native code field store unknown local 'ghost'"
        );
        assert_eq!(
            refused(
                builder.emit_field_owner_block(FieldContainer::State { resource: "ghost" }),
                "loading an unbound resource's STATE"
            ),
            "native code state assignment unknown local 'ghost'"
        );
        let slot = builder.allocate_stack_object("replacement", 8);
        assert_eq!(
            refused(
                builder.emit_field_owner_install(FieldContainer::Record { local: "ghost" }, slot),
                "installing into an unbound record local"
            ),
            "native code field store unknown local 'ghost'"
        );
    });
}

/// A bound record local is loaded from and installed into; a `STATE` payload is
/// never a replace owner, so installing over one is refused naming the rule.
#[test]
fn a_state_payload_is_never_a_replace_owner() {
    with_builder(|builder| {
        let stack_offset = builder.allocate_stack_object("p", 8);
        builder.locals.insert(
            "p".to_string(),
            LocalValue {
                type_: ParameterType::declared("Pt"),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        builder
            .emit_field_owner_block(FieldContainer::Record { local: "p" })
            .expect("a bound record local's block loads");
        let slot = builder.allocate_stack_object("replacement", 8);
        builder
            .emit_field_owner_install(FieldContainer::Record { local: "p" }, slot)
            .expect("a bound record local takes the replacement");
        assert_eq!(
            refused(
                builder.emit_field_owner_install(FieldContainer::State { resource: "h" }, slot),
                "installing over a STATE payload"
            ),
            "native field replace: 'h'.state is never a replace owner (replace_field_admitted)"
        );
    });
}
