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
//! `process` is a fully data-only package: every call's return type is fixed per
//! name (the overloading is on argument shape, not return), and no overload uses
//! an argument *union*, so `DefaultResolver::resolve_call`'s exact per-position
//! match answers arity/return/validation without a custom resolver.
//!
//! Landing across plan-90: **A** the plumbing + spawn/shell/pid/isRunning/
//! waitFor/close; **B** streaming I/O; **C** signals & detach; **D** Windows.

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinType, DefaultResolver,
    DefaultValue, Implementation, Lowering, Parameter, ParameterType, ReturnType, TypeKind,
};

/// The opaque `Process` resource handle type name.
pub(crate) const PROCESS_TYPE: &str = "Process";

const SPAWN: &str = "process.spawn";
const SHELL: &str = "process.shell";
const PID: &str = "process.pid";
const IS_RUNNING: &str = "process.isRunning";
const WAIT_FOR: &str = "process.waitFor";
const CLOSE: &str = "process.close";

/// The internal scope-drop op registered as `Process`'s resource close function.
///
/// Not user-callable: when a live `Process` goes out of scope the runtime
/// force-kills it (`SIGKILL`) and reaps it (`waitpid`) so no zombie is left and
/// drop never blocks. This is deliberately NOT the public `process::close`
/// (which closes only the child's stdin and leaves the child running) — so
/// `process::close(p)` is not treated as an ownership transfer and scope-drop
/// still runs `__drop`.
pub(crate) const DROP: &str = "process.__drop";

const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn nf(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const fn req(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}

// `spawn` has two structurally distinct overloads: the bare argv, and the full
// form adding a working directory, an environment map, and a replace-vs-merge
// flag. Both return a `Process`.
const P_SPAWN_ARGV: &[Parameter] = &[req("args", &[], "List OF String")];
const P_SPAWN_FULL: &[Parameter] = &[
    req("args", &[], "List OF String"),
    req("cwd", &[], "String"),
    req("env", &[], "Map OF String TO String"),
    req("envReplace", &[], "Boolean"),
];
const P_SHELL: &[Parameter] = &[req("cmd", &["command"], "String")];
// The lifecycle queries all take the `Process` receiver.
const P_PROC: &[Parameter] = &[req("p", &["process"], PROCESS_TYPE)];

const OV_SPAWN: &[BuiltinOverload] = &[
    ov(P_SPAWN_ARGV, PROCESS_TYPE),
    ov(P_SPAWN_FULL, PROCESS_TYPE),
];

const PROCESS_FUNCTIONS: &[BuiltinFunction] = &[
    nf(SPAWN, "spawn", OV_SPAWN),
    nf(SHELL, "shell", &[ov(P_SHELL, PROCESS_TYPE)]),
    nf(PID, "pid", &[ov(P_PROC, "Integer")]),
    nf(IS_RUNNING, "isRunning", &[ov(P_PROC, "Boolean")]),
    nf(WAIT_FOR, "waitFor", &[ov(P_PROC, "Integer")]),
    nf(CLOSE, "close", &[ov(P_PROC, "Nothing")]),
];

const PROCESS_TYPES: &[BuiltinType] = &[BuiltinType {
    name: PROCESS_TYPE,
    kind: TypeKind::Opaque,
    fields: &[],
}];

pub(crate) static PROCESS: BuiltinModule = BuiltinModule {
    name: "process",
    functions: PROCESS_FUNCTIONS,
    types: PROCESS_TYPES,
    // No source companion yet: `Process` is opaque (descriptor-only, per the
    // net/audio idiom); the `Stream`/`Signal` enums add a companion in B/C.
    source: None,
    // Fully data-only: `DefaultResolver` answers every metadata question.
    resolver: None,
};

/// Whether `name` is a public `process` builtin call (`process.spawn`, …). The
/// internal `__drop` op is not a descriptor call and is handled by the code layer
/// directly, so it is excluded here; use [`is_process_runtime_call`] for the
/// runtime-helper dispatch that includes it.
pub(crate) fn is_process_call(name: &str) -> bool {
    DefaultResolver::contains(&PROCESS, name)
}

/// Whether `name` is a `process` call that lowers to a `_mfb_rt_process_*`
/// runtime helper — every public call plus the internal `__drop` cleanup op.
pub(crate) fn is_process_runtime_call(name: &str) -> bool {
    is_process_call(name) || name == DROP
}

/// A bespoke expected-argument phrasing for `spawn`, whose two overloads have
/// structurally different positional layouts. The descriptor's per-position
/// render only shows the FIRST overload (`List OF String`), which mis-describes a
/// wrong 4-arg call; this `"or"`-joined string names both forms (the net/audio
/// idiom for an overloaded call). Every other `process` call has a single
/// signature the descriptor renders correctly, so this returns `None` for them
/// and they fall through to `DefaultResolver::expected_arguments`.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        SPAWN => {
            Some("List OF String or List OF String, String, Map OF String TO String, Boolean")
        }
        _ => None,
    }
}

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

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ret(name: &str, args: &[&str]) -> Option<String> {
        DefaultResolver::resolve_call(&PROCESS, name, &strings(args)).map(str::to_string)
    }

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
        assert_eq!(PROCESS.functions.len(), 6);
        assert!(PROCESS.source.is_none());
        assert!(PROCESS.resolver.is_none());
    }

    #[test]
    fn call_membership() {
        for f in [SPAWN, SHELL, PID, IS_RUNNING, WAIT_FOR, CLOSE] {
            assert!(DefaultResolver::contains(&PROCESS, f), "{f}");
        }
        // `__drop` is the internal scope-drop op, not a descriptor call.
        assert!(!DefaultResolver::contains(&PROCESS, DROP));
        assert!(!DefaultResolver::contains(&PROCESS, "process.bogus"));
    }

    #[test]
    fn spawn_overloads_resolve() {
        assert_eq!(ret(SPAWN, &["List OF String"]), Some("Process".to_string()));
        assert_eq!(
            ret(
                SPAWN,
                &["List OF String", "String", "Map OF String TO String", "Boolean"]
            ),
            Some("Process".to_string())
        );
        // Wrong arity / wrong types are rejected.
        assert_eq!(ret(SPAWN, &[]), None);
        assert_eq!(ret(SPAWN, &["String"]), None);
        assert_eq!(ret(SPAWN, &["List OF String", "String"]), None);
        assert_eq!(
            ret(
                SPAWN,
                &["List OF String", "String", "Map OF String TO String", "Integer"]
            ),
            None
        );
    }

    #[test]
    fn lifecycle_return_types() {
        assert_eq!(ret(SHELL, &["String"]), Some("Process".to_string()));
        assert_eq!(ret(PID, &["Process"]), Some("Integer".to_string()));
        assert_eq!(ret(IS_RUNNING, &["Process"]), Some("Boolean".to_string()));
        assert_eq!(ret(WAIT_FOR, &["Process"]), Some("Integer".to_string()));
        assert_eq!(ret(CLOSE, &["Process"]), Some("Nothing".to_string()));
        // Wrong receiver type is rejected.
        assert_eq!(ret(PID, &["Integer"]), None);
        assert_eq!(ret(CLOSE, &[]), None);
        assert_eq!(ret(SHELL, &["Integer"]), None);
    }

    #[test]
    fn arity_ranges() {
        assert_eq!(DefaultResolver::arity(&PROCESS, SPAWN), Some((1, 4)));
        assert_eq!(DefaultResolver::arity(&PROCESS, SHELL), Some((1, 1)));
        assert_eq!(DefaultResolver::arity(&PROCESS, PID), Some((1, 1)));
    }

    #[test]
    fn spawn_expected_arguments_names_both_overloads() {
        let text = expected_arguments(SPAWN).expect("spawn phrasing");
        assert!(text.contains("List OF String"));
        assert!(text.contains(" or "));
        assert!(text.contains("Boolean"));
        // Single-signature calls fall through to the descriptor renderer.
        assert_eq!(expected_arguments(SHELL), None);
        assert_eq!(expected_arguments(CLOSE), None);
    }

    #[test]
    fn return_type_name_is_fixed() {
        assert_eq!(
            DefaultResolver::return_type_name(&PROCESS, SPAWN),
            Some("Process")
        );
        assert_eq!(
            DefaultResolver::return_type_name(&PROCESS, CLOSE),
            Some("Nothing")
        );
    }
}
