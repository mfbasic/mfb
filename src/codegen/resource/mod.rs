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

use std::collections::HashMap;

use crate::codegen::registry::{registry, ResolvedType};

/// Where a resource registration came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    /// A standard built-in resource (`File`, `Socket`, `Listener`).
    Builtin,
    /// A resource contributed by an imported package's `RESOURCE_TABLE`.
    Imported,
    /// A native resource declared in this package by a `LINK` block
    /// `RESOURCE … CLOSE BY …` declaration (plan-link-update.md §9).
    Native,
}

/// Static description of a single resource type.
#[derive(Clone, Debug)]
pub(crate) struct ResourceInfo {
    /// The registered close op: a built-in call name like `"fs.close"`, or an
    /// imported package's close function name.
    pub close_function: String,
    /// Whether the resource may cross a thread boundary (the `RESOURCE_TABLE`
    /// "sendable to thread" bit).
    pub sendable: bool,
    /// Whether the close op can fail.
    ///
    /// Recorded at registration and not read by the compiler: drop-time cleanup
    /// handling derives the same fact independently, from whether the close
    /// wrapper declares `SUCCESS ON` (`ir::lower::…`, `ir::link::ResourceRecord::
    /// close_may_fail`). Kept because it is part of what a `RESOURCE_TABLE` row
    /// states, and dropping it would make this struct a lossy copy of the table.
    #[allow(dead_code)]
    pub close_may_fail: bool,
    /// Provenance of the registration. Read only by this module's own
    /// `every_builtin_resource_has_a_close_op` guard, which is the point: it is
    /// how a built-in seed is told apart from a package contribution if the two
    /// tables ever drift.
    #[allow(dead_code)]
    pub kind: ResourceKind,
}

/// Dynamic, data-driven table of resource types keyed by resolved type name.
///
/// Built once per compilation: seeded with [`ResourceRegistry::with_builtins`]
/// and then extended with each imported package's resources. Consulted wherever
/// the compiler needs to know whether a type is a resource, how to close it, or
/// whether it can be transferred across threads.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceRegistry {
    entries: HashMap<String, ResourceInfo>,
}

impl ResourceRegistry {
    /// A registry seeded with the standard built-in resources, read from the
    /// clean-room registry (`crate::codegen::registry`) — the single source of
    /// truth for every built-in resource's close op, sendability, and provenance.
    pub(crate) fn with_builtins() -> Self {
        let mut entries = HashMap::new();
        for pkg in registry().packages() {
            for r in pkg.resources() {
                entries.insert(
                    // The package-qualified type identity (`fs.File`, `net.Socket`).
                    format!("{}.{}", pkg.import_name(), r.name),
                    ResourceInfo {
                        close_function: r.close_function.to_string(),
                        sendable: r.sendable,
                        close_may_fail: r.close_may_fail,
                        kind: r.kind,
                    },
                );
            }
        }
        Self { entries }
    }

    /// Register (or override) a resource type.
    pub(crate) fn register(&mut self, type_name: impl Into<String>, info: ResourceInfo) {
        self.entries.insert(type_name.into(), info);
    }

    /// Whether `type_name` is a known resource type.
    pub(crate) fn is_resource(&self, type_name: &str) -> bool {
        self.entries.contains_key(type_name)
    }

    /// The registered close op for `type_name`, if it is a resource. Used by the
    /// type checker to recognize a close call as a transfer (`syntaxcheck::types`).
    pub(crate) fn close_function(&self, type_name: &str) -> Option<&str> {
        self.entries
            .get(type_name)
            .map(|info| info.close_function.as_str())
    }

    /// Whether `type_name` is a resource that may cross a thread boundary.
    pub(crate) fn is_sendable(&self, type_name: &str) -> bool {
        self.entries
            .get(type_name)
            .is_some_and(|info| info.sendable)
    }

    /// Whether closing `type_name` can fail.
    ///
    /// Test-only, and deliberately so: the compiler derives this fact from the
    /// close wrapper's `SUCCESS ON` clause rather than from here (see
    /// [`ResourceInfo::close_may_fail`]). It survives as the only way
    /// `builtins_carry_close_op_and_sendability` can assert what the built-in
    /// seed table records, so the seed cannot drift unnoticed.
    #[cfg(test)]
    pub(crate) fn close_may_fail(&self, type_name: &str) -> bool {
        self.entries
            .get(type_name)
            .is_some_and(|info| info.close_may_fail)
    }
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
        let registry = ResourceRegistry::with_builtins();
        assert!(registry.is_resource("fs.File"));
        assert!(registry.is_resource("net.Socket"));
        assert!(registry.is_resource("net.Listener"));
        assert!(!registry.is_resource("Integer"));
        assert!(!registry.is_resource("Address"));
    }

    #[test]
    fn builtins_carry_close_op_and_sendability() {
        let registry = ResourceRegistry::with_builtins();
        assert_eq!(registry.close_function("fs.File"), Some("fs.close"));
        assert_eq!(registry.close_function("net.Socket"), Some("net.close"));
        assert_eq!(registry.close_function("net.Listener"), Some("net.close"));
        // File and Socket move across threads; a Listener stays put.
        assert!(registry.is_sendable("fs.File"));
        assert!(registry.is_sendable("net.Socket"));
        assert!(!registry.is_sendable("net.Listener"));
        // close-may-fail holds for every standard resource.
        assert!(registry.close_may_fail("fs.File"));
        assert!(registry.close_may_fail("net.Listener"));
    }

    #[test]
    fn imported_resource_registers_and_does_not_disturb_builtins() {
        let mut registry = ResourceRegistry::with_builtins();
        registry.register(
            "DbHandle",
            ResourceInfo {
                close_function: "db.close".to_string(),
                sendable: false,
                close_may_fail: true,
                kind: ResourceKind::Imported,
            },
        );
        assert!(registry.is_resource("DbHandle"));
        assert_eq!(registry.close_function("DbHandle"), Some("db.close"));
        assert!(!registry.is_sendable("DbHandle"));
        // Built-ins remain intact.
        assert!(registry.is_sendable("fs.File"));
    }

    #[test]
    fn every_builtin_resource_has_a_close_op() {
        // The closed-default (plan-38) relies on every built-in resource being
        // closeable so scope-drop can no-op a closed-default record. Guard against
        // a new built-in added without a registered close op (which would also
        // need a closed-flag review at the canonical offset 8).
        let registry = ResourceRegistry::with_builtins();
        for (name, info) in &registry.entries {
            assert_eq!(info.kind, ResourceKind::Builtin, "{name} must be Builtin");
            assert!(
                !info.close_function.is_empty(),
                "{name} has an empty close op"
            );
        }
        // The full set of built-ins the closed-default must cover.
        for name in [
            // All built-in resources carry their package-qualified identity (plan-97).
            "fs.File",
            "net.Socket",
            "net.Listener",
            "net.UdpSocket",
            "audio.AudioInput",
            "audio.AudioOutput",
            "tls.TlsSocket",
            "tls.TlsListener",
            "process.Process",
        ] {
            assert!(registry.is_resource(name), "{name} missing from registry");
            assert!(
                registry.close_function(name).is_some_and(|c| !c.is_empty()),
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
