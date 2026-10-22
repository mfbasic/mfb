//! `runtime::helper_for_call`: which runtime-helper family a call is routed
//! to. The validator's capability walk and the lowering's runtime-call path
//! both key on this answer, so a member landing in the wrong family (or in
//! none) is refused or mis-lowered on every backend.

use crate::target::shared::runtime::{helper_for_call, RuntimeHelper};

/// Every registered member of a package whose family is its own package name
/// routes to that family — whichever branch of the classifier claims it.
#[test]
fn every_member_of_a_runtime_package_routes_to_its_package_family() {
    for (package, family) in [
        ("app", RuntimeHelper::App),
        ("io", RuntimeHelper::Io),
        ("thread", RuntimeHelper::Thread),
        ("tls", RuntimeHelper::Tls),
        ("math", RuntimeHelper::Math),
        ("term", RuntimeHelper::Term),
        ("net", RuntimeHelper::Net),
        ("os", RuntimeHelper::Os),
        ("process", RuntimeHelper::Process),
    ] {
        let members = crate::codegen::registry::registry()
            .resolve_package(package)
            .unwrap_or_else(|| panic!("{package} is registered"))
            .functions();
        assert!(!members.is_empty(), "{package} has members");
        for function in members {
            let call = format!("{package}.{}", function.name);
            assert_eq!(helper_for_call(&call), Some(family), "{call}");
        }
    }
}

/// The calls IR lowering synthesizes, which no package declares, still route
/// to the family whose runtime serves them.
#[test]
fn synthesized_runtime_calls_route_to_their_serving_family() {
    for call in [
        "thread.transferResource",
        "thread.acceptResource",
        "thread.emitResource",
        "thread.readResource",
    ] {
        assert_eq!(helper_for_call(call), Some(RuntimeHelper::Thread), "{call}");
    }
    assert_eq!(
        helper_for_call("tls.closeListener"),
        Some(RuntimeHelper::Tls)
    );
    assert_eq!(
        helper_for_call("process.__drop"),
        Some(RuntimeHelper::Process)
    );
    // A user function is no runtime call at all.
    assert_eq!(helper_for_call("main"), None);
}
