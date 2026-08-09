//! The built-in `process` package (plan-90).
//!
//! `process` spawns and manages child processes. Its one resource, `Process`, is
//! a native resource (tag 10) whose 96-byte record tail holds the three pipe fds
//! (stdin-write / stdout-read / stderr-read) plus the cached exit/signal state.
//!
//! Like `net`/`audio`, the callable metadata (arity, return types, overload
//! selection) lives here in the front end and the OS mechanism (fork/exec/pipe/
//! waitpid on Unix, `CreateProcess` on Windows) is emitted by the native backend
//! in `src/target/shared/code/process/`. The opaque `Process` handle is declared
//! only in the descriptor `types` list below (the net/audio idiom for an opaque
//! resource — a companion `.mfb` carries value records/enums, never the handle).
//!
//! plan-90-A lands the package plumbing and the opaque `Process` type; the
//! callable surface and native backend land in the later phases/sub-plans.

use super::descriptor::{BuiltinModule, BuiltinType, TypeKind};

/// The opaque `Process` resource handle type name.
pub(crate) const PROCESS_TYPE: &str = "Process";

/// The internal scope-drop op registered as `Process`'s resource close function.
///
/// Not user-callable: when a live `Process` goes out of scope the runtime
/// force-kills it (`SIGKILL`) and reaps it (`waitpid`) so no zombie is left and
/// drop never blocks. This is deliberately NOT the public `process::close`
/// (which closes only the child's stdin and leaves the child running) — so
/// `process::close(p)` is not treated as an ownership transfer and scope-drop
/// still runs `__drop`.
pub(crate) const DROP: &str = "process.__drop";

const PROCESS_TYPES: &[BuiltinType] = &[BuiltinType {
    name: PROCESS_TYPE,
    kind: TypeKind::Opaque,
    fields: &[],
}];

pub(crate) static PROCESS: BuiltinModule = BuiltinModule {
    name: "process",
    // The callable surface lands in plan-90-A Phase 2 and sub-plans B/C.
    functions: &[],
    types: PROCESS_TYPES,
    // No source companion yet: `Process` is opaque (descriptor-only, per the
    // net/audio idiom); the `Stream`/`Signal` enums add a companion in B/C.
    source: None,
    resolver: None,
};

/// Whether `name` is a `process` value/opaque type (`Process`).
pub(crate) fn is_builtin_type(name: &str) -> bool {
    PROCESS.types.iter().any(|ty| ty.name == name)
}

/// The scope-drop close op for a `process` resource type, if any. `Process` is
/// reaped via the internal `__drop` op (SIGKILL + waitpid), not the public
/// `close`.
pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        PROCESS_TYPE => Some(DROP),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_is_a_builtin_opaque_type() {
        assert!(is_builtin_type(PROCESS_TYPE));
        assert!(!is_builtin_type("Nothing"));
        assert!(!is_builtin_type("Socket"));
        let ty = PROCESS
            .types
            .iter()
            .find(|t| t.name == PROCESS_TYPE)
            .expect("Process registered");
        assert_eq!(ty.kind, TypeKind::Opaque);
        assert!(ty.fields.is_empty());
    }

    #[test]
    fn process_close_op_is_drop() {
        assert_eq!(resource_close_function(PROCESS_TYPE), Some(DROP));
        assert_eq!(resource_close_function("Nothing"), None);
    }

    #[test]
    fn module_shape() {
        assert_eq!(PROCESS.name, "process");
        assert!(PROCESS.functions.is_empty());
        assert!(PROCESS.source.is_none());
        assert!(PROCESS.resolver.is_none());
    }
}
