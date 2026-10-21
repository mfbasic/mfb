//! Which runtime helper family owns a call.
//!
//! A runtime call's helper decides which family's emitters the module carries and
//! which imports its plan declares; a call routed to the wrong family (or to none)
//! is an undefined symbol at link time. Each family is pinned by a call it owns.

use crate::target::shared::runtime::{helper_for_call, RuntimeHelper};

#[test]
fn each_package_s_calls_route_to_that_package_s_helper() {
    for (call, helper) in [
        ("app.setMode", RuntimeHelper::App),
        ("io.print", RuntimeHelper::Io),
        ("math.sqrt", RuntimeHelper::Math),
        ("term.on", RuntimeHelper::Term),
        ("thread.start", RuntimeHelper::Thread),
        ("net.lookup", RuntimeHelper::Net),
        ("os.exit", RuntimeHelper::Os),
        ("process.__drop", RuntimeHelper::Process),
        ("tls.closeListener", RuntimeHelper::Tls),
    ] {
        assert_eq!(helper_for_call(call), Some(helper), "{call}");
    }
}

#[test]
fn a_call_no_family_owns_routes_to_no_helper() {
    assert_eq!(helper_for_call("nosuch.call"), None);
}

#[test]
fn every_helper_is_named_for_its_package() {
    for (helper, name) in [
        (RuntimeHelper::App, "app"),
        (RuntimeHelper::Math, "math"),
        (RuntimeHelper::Tls, "tls"),
        (RuntimeHelper::Udp, "udp"),
        (RuntimeHelper::Perf, "perf"),
    ] {
        assert_eq!(helper.name(), name);
    }
}
