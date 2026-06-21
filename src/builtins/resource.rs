//! Data-driven resource registry.
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

use std::collections::HashMap;
use std::sync::LazyLock;

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
//
// `close_may_fail` and `kind` are recorded by the registry now and consumed by
// later overhaul phases (drop-time cleanup handling and resource-union/thread
// classification); allow them to sit unread until then.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ResourceInfo {
    /// The registered close op: a built-in call name like `"fs.close"`, or an
    /// imported package's close function name.
    pub close_function: String,
    /// Whether the resource may cross a thread boundary (the `RESOURCE_TABLE`
    /// "sendable to thread" bit).
    pub sendable: bool,
    /// Whether the close op can fail (drives drop-time cleanup handling).
    pub close_may_fail: bool,
    /// Provenance of the registration.
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
    /// A registry seeded with the standard built-in resources.
    pub(crate) fn with_builtins() -> Self {
        Self {
            entries: builtin_resources().clone(),
        }
    }

    /// Register (or override) a resource type.
    pub(crate) fn register(&mut self, type_name: impl Into<String>, info: ResourceInfo) {
        self.entries.insert(type_name.into(), info);
    }

    /// Whether `type_name` is a known resource type.
    pub(crate) fn is_resource(&self, type_name: &str) -> bool {
        self.entries.contains_key(type_name)
    }

    /// The full registration for `type_name`, if any. Consumed by later phases
    /// (LUT runtime, resource unions); kept here as the registry's primary API.
    #[allow(dead_code)]
    pub(crate) fn info(&self, type_name: &str) -> Option<&ResourceInfo> {
        self.entries.get(type_name)
    }

    /// The registered close op for `type_name`, if it is a resource. Consumed by
    /// the LUT runtime phase for drop/close lowering.
    #[allow(dead_code)]
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

    /// Whether closing `type_name` can fail. Consumed by the LUT runtime phase
    /// for the drop-time cleanup-failure ledger.
    #[allow(dead_code)]
    pub(crate) fn close_may_fail(&self, type_name: &str) -> bool {
        self.entries
            .get(type_name)
            .is_some_and(|info| info.close_may_fail)
    }
}

/// The standard built-in resources, the single source of truth for both the
/// dynamic [`ResourceRegistry`] and the free `is_builtin_*` helpers.
static BUILTIN_RESOURCES: LazyLock<HashMap<String, ResourceInfo>> = LazyLock::new(|| {
    let mut entries = HashMap::new();
    entries.insert(
        super::fs::FILE_TYPE.to_string(),
        ResourceInfo {
            close_function: super::fs::resource_close_function(super::fs::FILE_TYPE)
                .expect("File has a built-in close op")
                .to_string(),
            sendable: true,
            close_may_fail: true,
            kind: ResourceKind::Builtin,
        },
    );
    entries.insert(
        super::net::SOCKET_TYPE.to_string(),
        ResourceInfo {
            close_function: super::net::resource_close_function(super::net::SOCKET_TYPE)
                .expect("Socket has a built-in close op")
                .to_string(),
            sendable: true,
            close_may_fail: true,
            kind: ResourceKind::Builtin,
        },
    );
    entries.insert(
        super::net::LISTENER_TYPE.to_string(),
        ResourceInfo {
            close_function: super::net::resource_close_function(super::net::LISTENER_TYPE)
                .expect("Listener has a built-in close op")
                .to_string(),
            // A listener accepts connections on the owning thread; it is not
            // moved across thread boundaries.
            sendable: false,
            close_may_fail: true,
            kind: ResourceKind::Builtin,
        },
    );
    entries
});

fn builtin_resources() -> &'static HashMap<String, ResourceInfo> {
    &BUILTIN_RESOURCES
}

/// The bare resource type name, with any `STATE T` suffix removed. A stateful
/// resource carries its `STATE` type in the type string (`File STATE FileState`)
/// once lowered to IR/NIR; recognition keys on the bare resource name.
pub(crate) fn base_resource_name(type_name: &str) -> &str {
    match type_name.split_once(" STATE ") {
        Some((base, _)) => base,
        None => type_name,
    }
}

/// The `STATE` record type carried by a resource type string, if any.
pub(crate) fn state_type_name(type_name: &str) -> Option<&str> {
    type_name.split_once(" STATE ").map(|(_, state)| state)
}

/// Whether `type_name` is a built-in resource type. Used by stages that only
/// ever see built-in resources (codegen, binary-representation writer).
pub(crate) fn is_builtin_resource_type(type_name: &str) -> bool {
    BUILTIN_RESOURCES.contains_key(base_resource_name(type_name))
}

/// The built-in close op for `type_name`, if it is a built-in resource.
pub(crate) fn builtin_resource_close_function(type_name: &str) -> Option<&'static str> {
    BUILTIN_RESOURCES
        .get(base_resource_name(type_name))
        .map(|info| info.close_function.as_str())
}

/// Whether `type_name` is a built-in resource that may cross a thread boundary.
pub(crate) fn is_builtin_sendable_resource_type(type_name: &str) -> bool {
    BUILTIN_RESOURCES
        .get(base_resource_name(type_name))
        .is_some_and(|info| info.sendable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_recognize_standard_resources() {
        let registry = ResourceRegistry::with_builtins();
        assert!(registry.is_resource("File"));
        assert!(registry.is_resource("Socket"));
        assert!(registry.is_resource("Listener"));
        assert!(!registry.is_resource("Integer"));
        assert!(!registry.is_resource("Address"));
    }

    #[test]
    fn builtins_carry_close_op_and_sendability() {
        let registry = ResourceRegistry::with_builtins();
        assert_eq!(registry.close_function("File"), Some("fs.close"));
        assert_eq!(registry.close_function("Socket"), Some("net.close"));
        assert_eq!(registry.close_function("Listener"), Some("net.close"));
        // File and Socket move across threads; a Listener stays put.
        assert!(registry.is_sendable("File"));
        assert!(registry.is_sendable("Socket"));
        assert!(!registry.is_sendable("Listener"));
        // close-may-fail holds for every standard resource.
        assert!(registry.close_may_fail("File"));
        assert!(registry.close_may_fail("Listener"));
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
        assert!(registry.is_sendable("File"));
    }

    #[test]
    fn free_helpers_match_registry() {
        assert!(is_builtin_resource_type("File"));
        assert!(!is_builtin_resource_type("Nothing"));
        assert_eq!(builtin_resource_close_function("Socket"), Some("net.close"));
        assert!(is_builtin_sendable_resource_type("Socket"));
        assert!(!is_builtin_sendable_resource_type("Listener"));
    }
}
