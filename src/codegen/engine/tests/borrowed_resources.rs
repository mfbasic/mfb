//! `CodeBuilder::value_aliases_live_resource` (`engine/value/builder_values/ownership.rs`):
//! which `RES` bind sources are aliases of a resource something else owns.

use crate::codegen::engine::builder::CodeBuilder;
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::target::shared::runtime::RuntimeHelper;

fn runtime_call(target: &str) -> NirValue {
    NirValue::RuntimeCall {
        helper: RuntimeHelper::Tcp,
        target: target.to_string(),
        args: Vec::new(),
        loc: NirSourceLoc::default(),
    }
}

fn plain_call(target: &str) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args: Vec::new(),
        loc: NirSourceLoc::default(),
    }
}

/// plan-114-C: `RES g = h.handle` reads a handle back OUT of a record field.
/// The record's scope owns it, so the bind registers no close of its own —
/// the same rule bug-375 established for `poll(List)`, one container-kind
/// over. Registering a close here would release the record's handle at this
/// scope's exit and the record's drain would then close it again.
#[test]
fn a_record_field_read_aliases_a_live_resource() {
    let read = NirValue::MemberAccess {
        target: Box::new(NirValue::Local("h".to_string())),
        member: "handle".to_string(),
    };
    assert!(
        CodeBuilder::value_aliases_live_resource(&read),
        "a record field read is an alias, not a producer"
    );
}

/// The list form of `poll` returns a pointer to an element the list still
/// owns, so its `RES` bind must register no close (bug-375).
///
/// The guard is on the **NIR variant**, not just the name: a built-in
/// package's member lowers to `NirValue::RuntimeCall`, and this predicate
/// used to match only `Call`/`CallResult`. The `net.poll`/`tls.poll` names it
/// listed were therefore never reached, and every `poll(List)` bind was
/// classified as an owner -- closing the borrowed element at the bind's scope
/// exit and again when the list drained.
#[test]
fn poll_list_forms_alias_a_live_resource() {
    for target in ["tcp.poll", "udp.poll", "tls.poll"] {
        assert!(
            CodeBuilder::value_aliases_live_resource(&runtime_call(target)),
            "{target} returns a borrowed list element"
        );
    }
    // A producing member of the same package still transfers ownership.
    assert!(!CodeBuilder::value_aliases_live_resource(&runtime_call(
        "tcp.accept"
    )));
    // `collections::get`/`getOr` keep their own borrow classification, which
    // arrives as a plain call rather than a runtime call.
    assert!(CodeBuilder::value_aliases_live_resource(&plain_call(
        "collections.get"
    )));
}
