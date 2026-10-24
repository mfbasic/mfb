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
        ("term.measureTextRaw", RuntimeHelper::Term),
        ("thread.start", RuntimeHelper::Thread),
        ("net.lookup", RuntimeHelper::Net),
        ("net.toUrl", RuntimeHelper::Net),
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

/// The package -> family table (`RuntimeHelper::from_package_name`) is total
/// over the runtime calls the catalog derives without an `abi_function` body:
/// the registry resources' close ops. Every such call's package has a family,
/// and it is the family `helper_for_call` routes the call to, so the catalog's
/// lookup for them can never miss.
#[test]
fn a_registry_resource_close_op_s_package_has_a_family() {
    let close_ops: Vec<&str> = crate::codegen::registry::runtime_specs()
        .iter()
        .map(|call| call.name)
        .filter(|name| !crate::codegen::registry::is_abi_function_call(name))
        .collect();
    assert!(
        !close_ops.is_empty(),
        "the registry declares resource close ops"
    );
    for name in close_ops {
        let package = name.split_once('.').expect("package-qualified").0;
        let family = RuntimeHelper::from_package_name(package);
        assert!(
            family.is_some(),
            "{name}: package `{package}` has no RuntimeHelper"
        );
        assert_eq!(family, helper_for_call(name), "{name}");
    }
}
