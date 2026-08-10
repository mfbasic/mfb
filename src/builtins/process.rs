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
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, BuiltinType,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType, TypeKind,
};

/// The opaque `Process` resource handle type name.
pub(crate) const PROCESS_TYPE: &str = "Process";

/// The `Stream` enum (`StdOut`/`StdErr`) selecting which child pipe a read reads
/// from. Declared as an `EXPORT ENUM` in the source companion (plan-90-B).
pub(crate) const STREAM_TYPE: &str = "Stream";

/// The `Signal` enum (`None`/`Kill`/`Terminate`/`Error`) — the 4-bucket
/// send/observe vocabulary. Declared as an `EXPORT ENUM` in the companion
/// (plan-90-C).
pub(crate) const SIGNAL_TYPE: &str = "Signal";

const SPAWN: &str = "process.spawn";
const SHELL: &str = "process.shell";
const PID: &str = "process.pid";
const IS_RUNNING: &str = "process.isRunning";
const WAIT_FOR: &str = "process.waitFor";
const CLOSE: &str = "process.close";
// plan-90-B streaming I/O.
const SEND: &str = "process.send";
const SEND_BYTES: &str = "process.sendBytes";
const RECEIVE: &str = "process.receive";
const RECEIVE_BYTES: &str = "process.receiveBytes";
const POLL: &str = "process.poll";
// plan-90-C signals & detach.
const SIGNAL: &str = "process.signal";
const DID_SIGNAL: &str = "process.didSignal";
const DETACH: &str = "process.detach";

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
        doc_into: "",
        doc_desc: "",
        errors: &[],
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
// plan-90-B streaming I/O parameter lists.
const P_SEND: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("text", &[], "String"),
];
const P_SEND_T: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("text", &[], "String"),
    req("timeoutMs", &[], "Integer"),
];
const P_SENDB: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("data", &[], "List OF Byte"),
];
const P_SENDB_T: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("data", &[], "List OF Byte"),
    req("timeoutMs", &[], "Integer"),
];
const P_RECV: &[Parameter] = &[req("p", &["process"], PROCESS_TYPE)];
const P_RECV_S: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("from", &[], STREAM_TYPE),
];
const P_POLL: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("ms", &[], "Integer"),
];
const P_POLL_S: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("ms", &[], "Integer"),
    req("from", &[], STREAM_TYPE),
];

const OV_SEND: &[BuiltinOverload] = &[ov(P_SEND, "Nothing"), ov(P_SEND_T, "Nothing")];
const OV_SEND_BYTES: &[BuiltinOverload] = &[ov(P_SENDB, "Nothing"), ov(P_SENDB_T, "Nothing")];
const OV_RECEIVE: &[BuiltinOverload] = &[ov(P_RECV, "String"), ov(P_RECV_S, "String")];
const OV_RECEIVE_BYTES: &[BuiltinOverload] =
    &[ov(P_RECV, "List OF Byte"), ov(P_RECV_S, "List OF Byte")];
const OV_POLL: &[BuiltinOverload] = &[ov(P_POLL, "Boolean"), ov(P_POLL_S, "Boolean")];
const P_SIGNAL: &[Parameter] = &[
    req("p", &["process"], PROCESS_TYPE),
    req("sig", &["signal"], SIGNAL_TYPE),
];

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
    nf(SEND, "send", OV_SEND),
    nf(SEND_BYTES, "sendBytes", OV_SEND_BYTES),
    nf(RECEIVE, "receive", OV_RECEIVE),
    nf(RECEIVE_BYTES, "receiveBytes", OV_RECEIVE_BYTES),
    nf(POLL, "poll", OV_POLL),
    nf(SIGNAL, "signal", &[ov(P_SIGNAL, "Nothing")]),
    nf(DID_SIGNAL, "didSignal", &[ov(P_PROC, SIGNAL_TYPE)]),
    nf(DETACH, "detach", &[ov(P_PROC, "Nothing")]),
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
    // The source companion carries the `Stream` (plan-90-B) and `Signal`
    // (plan-90-C) value enums; the opaque `Process` handle stays descriptor-only.
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
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
        SPAWN => Some("List OF String or List OF String, String, Map OF String TO String, Boolean"),
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

super::package_source_glue!(
    "process",
    "<builtin-process>",
    "builtins/process.mfb",
    include_str!("process_package.mfb")
);

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn const_builders_populate_their_descriptors() {
        // `req`/`ov`/`nf` are `const fn` table builders, const-evaluated where the
        // static tables use them and thus otherwise uncovered. Drive them at
        // runtime with `black_box`'d ('static) inputs so the calls cannot be folded
        // back to consts, and assert they populate each descriptor field.
        use std::hint::black_box;
        const ALIASES: &[&str] = &["command"];

        let p = req(black_box("cmd"), black_box(ALIASES), black_box("String"));
        assert_eq!(p.name, "cmd");
        assert_eq!(p.aliases, ALIASES);
        assert!(matches!(p.ty, ParameterType::Named("String")));
        assert!(matches!(p.default, DefaultValue::None));

        let o = ov(black_box(P_SHELL), black_box("Boolean"));
        assert_eq!(o.params.len(), P_SHELL.len());
        assert!(matches!(o.return_type, ReturnType::Fixed("Boolean")));

        let f = nf(black_box("process.demo"), black_box("demo"), black_box(OV_POLL));
        assert_eq!(f.name, "process.demo");
        assert_eq!(f.doc_slug, "demo");
        assert_eq!(f.overloads.len(), OV_POLL.len());
        assert!(matches!(f.implementation, Implementation::Same));
        assert!(matches!(f.lowering, Lowering::Helper));
        assert!(!f.flags.internal_only);
        assert!(!f.flags.return_type_overloaded);
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
        assert_eq!(PROCESS.functions.len(), 14);
        assert!(PROCESS.source.is_some());
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
                &[
                    "List OF String",
                    "String",
                    "Map OF String TO String",
                    "Boolean"
                ]
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
                &[
                    "List OF String",
                    "String",
                    "Map OF String TO String",
                    "Integer"
                ]
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
        assert_eq!(DefaultResolver::arity(&PROCESS, SEND), Some((2, 3)));
        assert_eq!(DefaultResolver::arity(&PROCESS, RECEIVE), Some((1, 2)));
        assert_eq!(DefaultResolver::arity(&PROCESS, POLL), Some((2, 3)));
    }

    #[test]
    fn streaming_io_resolves() {
        assert_eq!(
            ret(SEND, &["Process", "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            ret(SEND, &["Process", "String", "Integer"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            ret(SEND_BYTES, &["Process", "List OF Byte"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(RECEIVE, &["Process"]), Some("String".to_string()));
        assert_eq!(
            ret(RECEIVE, &["Process", "Stream"]),
            Some("String".to_string())
        );
        assert_eq!(
            ret(RECEIVE_BYTES, &["Process"]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            ret(POLL, &["Process", "Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            ret(POLL, &["Process", "Integer", "Stream"]),
            Some("Boolean".to_string())
        );
        // Wrong types rejected.
        assert_eq!(ret(SEND, &["Process", "Integer"]), None);
        assert_eq!(ret(RECEIVE, &["Process", "Integer"]), None);
    }

    #[test]
    fn source_companion_parses() {
        assert!(source_file().is_ok());
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
