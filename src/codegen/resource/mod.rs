//! `codegen::resource` module wiring, plus the data-driven resource registry.
//!
//! Resources used to be recognized by a hardcoded set of type names spread
//! across the type checker, the binary-representation writer, and the backend.
//! This module replaces that with a single table keyed by resolved type name.
//! It is seeded with the standard built-ins (`File`, `Socket`, `Listener`) and
//! extended at type-check time from each imported package's `RESOURCE_TABLE`.
//!
//! Stages that operate on already-resolved types and therefore only ever see
//! built-in resources (the backend, the binary-representation writer) keep using
//! the free `is_builtin_*` helpers, which read from the same built-in table so
//! there is one source of truth.
//!
//! (Relocated from `src/builtins/resource.rs` into the codegen layer it fronts,
//! plan-103. Pure code motion.)

pub(crate) mod cleanup;

use crate::codegen::registry::{registry, ResolvedType};

/// Where a resource descriptor came from. Only built-in resources carry a
/// descriptor (the clean-room registry's); a project's native `LINK` resources
/// and an imported package's `RESOURCE_TABLE` rows are read by `ir::shape` /
/// `ir::verify` directly (plan-107-D deleted the source checker's registry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    /// A standard built-in resource (`File`, `Socket`, `Listener`).
    Builtin,
}

/// Split a resource type string at its **own** top-level `STATE` clause, if any.
/// A bare stateful resource is spelled `<ResourceName> STATE <StateType>`, where
/// `<ResourceName>` is a single type token (a bare name or a `pkg.Name`, never
/// containing a space). A `STATE` nested inside a composite type — a thread
/// plane's `RES` element (`Thread OF RES File STATE Cursor TO Out`, plan-54), or
/// a `List`/`Map` of a stateful resource — is the inner resource's state, not the
/// composite's own, so it must NOT be split off here (doing so truncated
/// `ThreadWorker OF RES File STATE Cursor TO Integer` to `ThreadWorker OF RES
/// File`). Keying on a space in the base distinguishes the two.
fn split_state_clause(type_name: &str) -> Option<(&str, &str)> {
    let (base, state) = type_name.split_once(" STATE ")?;
    if base.contains(' ') {
        return None; // nested STATE inside a composite type — not this type's own.
    }
    Some((base, state))
}

/// The bare resource type name, with any `STATE T` suffix removed. A stateful
/// resource carries its `STATE` type in the type string (`File STATE FileState`)
/// once lowered to IR/NIR; recognition keys on the bare resource name.
pub(crate) fn base_resource_name(type_name: &str) -> &str {
    match split_state_clause(type_name) {
        Some((base, _)) => base,
        None => type_name,
    }
}

/// The `STATE` record type carried by a resource type string, if any.
pub(crate) fn state_type_name(type_name: &str) -> Option<&str> {
    split_state_clause(type_name).map(|(_, state)| state)
}

/// Whether `type_name` is a built-in resource type. Used by stages that only
/// ever see built-in resources (codegen, binary-representation writer).
pub(crate) fn is_builtin_resource_type(type_name: &str) -> bool {
    matches!(
        registry().resolve_type(base_resource_name(type_name)),
        Some(ResolvedType::Resource(_))
    )
}

/// Whether `type_name` names — or is the **bare base** of — a built-in resource
/// (`File` matches the package-qualified `fs.File`). Used ONLY on the package-import
/// path (`collect_package_resources` / `imported_resource_closers`): an imported
/// `.mfp` may reference a builtin resource by its bare base name even though the
/// builtin's own identity is package-qualified (plan-97). Deliberately more lenient
/// than [`is_builtin_resource_type`] — it must NOT be used for user-type resolution,
/// where a bare `File` is a distinct user type.
pub(crate) fn is_builtin_backed_resource(type_name: &str) -> bool {
    if is_builtin_resource_type(type_name) {
        return true;
    }
    let bare = base_resource_name(type_name)
        .rsplit('.')
        .next()
        .unwrap_or(type_name);
    registry()
        .packages()
        .iter()
        .any(|p| p.resources().iter().any(|r| r.name == bare))
}

/// The built-in close op for `type_name`, if it is a built-in resource.
pub(crate) fn builtin_resource_close_function(type_name: &str) -> Option<&'static str> {
    match registry().resolve_type(base_resource_name(type_name)) {
        Some(ResolvedType::Resource(r)) => Some(r.close_function),
        _ => None,
    }
}

/// Whether `type_name` is a built-in resource that may cross a thread boundary.
pub(crate) fn is_builtin_sendable_resource_type(type_name: &str) -> bool {
    match registry().resolve_type(base_resource_name(type_name)) {
        Some(ResolvedType::Resource(r)) => r.sendable,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_recognize_standard_resources() {
        assert!(is_builtin_resource_type("fs.File"));
        assert!(is_builtin_resource_type("net.Socket"));
        assert!(is_builtin_resource_type("net.Listener"));
        assert!(!is_builtin_resource_type("Integer"));
        assert!(!is_builtin_resource_type("Address"));
    }

    #[test]
    fn builtins_carry_close_op_and_sendability() {
        let descriptor = |name: &str| match registry().resolve_type(name) {
            Some(ResolvedType::Resource(r)) => r,
            _ => panic!("{name} is not a built-in resource"),
        };
        assert_eq!(builtin_resource_close_function("fs.File"), Some("fs.close"));
        assert_eq!(
            builtin_resource_close_function("net.Socket"),
            Some("net.close")
        );
        assert_eq!(
            builtin_resource_close_function("net.Listener"),
            Some("net.close")
        );
        // File and Socket move across threads; a Listener stays put.
        assert!(is_builtin_sendable_resource_type("fs.File"));
        assert!(is_builtin_sendable_resource_type("net.Socket"));
        assert!(!is_builtin_sendable_resource_type("net.Listener"));
        // close-may-fail holds for every standard resource (the descriptor
        // states it; drop-time cleanup derives the same fact from the close
        // wrapper's `SUCCESS ON`).
        assert!(descriptor("fs.File").close_may_fail);
        assert!(descriptor("net.Listener").close_may_fail);
    }

    #[test]
    fn every_builtin_resource_has_a_close_op() {
        // The closed-default (plan-38) relies on every built-in resource being
        // closeable so scope-drop can no-op a closed-default record. Guard against
        // a new built-in added without a registered close op (which would also
        // need a closed-flag review at the canonical offset 8).
        for pkg in registry().packages() {
            for r in pkg.resources() {
                let name = format!("{}.{}", pkg.import_name(), r.name);
                assert_eq!(r.kind, ResourceKind::Builtin, "{name} must be Builtin");
                assert!(!r.close_function.is_empty(), "{name} has an empty close op");
            }
        }
        // The full set of built-ins the closed-default must cover.
        for name in [
            // All built-in resources carry their package-qualified identity (plan-97).
            "fs.File",
            "net.Socket",
            "net.Listener",
            // plan-110-B/C: the transport handles moved out of `net`.
            // `net.UdpSocket` is gone entirely — `udp.Socket` replaces it.
            "tcp.Socket",
            "tcp.Listener",
            "udp.Socket",
            "audio.AudioInput",
            "audio.AudioOutput",
            "tls.TlsSocket",
            "tls.TlsListener",
            "process.Process",
        ] {
            assert!(
                is_builtin_resource_type(name),
                "{name} missing from registry"
            );
            assert!(
                builtin_resource_close_function(name).is_some_and(|c| !c.is_empty()),
                "{name} has no close op"
            );
        }
    }

    #[test]
    fn free_helpers_match_registry() {
        assert!(is_builtin_resource_type("fs.File"));
        assert!(!is_builtin_resource_type("Nothing"));
        assert_eq!(
            builtin_resource_close_function("net.Socket"),
            Some("net.close")
        );
        assert!(is_builtin_sendable_resource_type("net.Socket"));
        assert!(!is_builtin_sendable_resource_type("net.Listener"));
    }
}
