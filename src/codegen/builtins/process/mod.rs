//! The built-in `process` package (plan-90).
//!
//! `process` spawns and manages child processes. Its one resource, `Process`, is
//! a native resource (tag 10) whose 96-byte record tail holds the three pipe fds
//! (stdin-write / stdout-read / stderr-read) plus the cached exit/signal state.
//!
//! `process` is a **native** OS-seam package migrated onto the clean-room registry
//! (`crate::codegen::registry`) and onto the `Body::abi_function` clean-room shape
//! (crypto/io/fs/net). Each member owns its `Body::abi_function` body in its own
//! `func_*.rs` (`lower_<name>`), which branches the posix/win backend by
//! `platform.family()` and calls the member's own `lower_process_<name>_helper_{win,
//! posix}` emitter (also in the `func_*.rs`, delegating to the arch-neutral
//! `gen_{unix,windows}` emission), with its `spawnEnv`/`sendTimeout`/… code-form
//! alias distinguished off [`AbiCtx::call`](crate::codegen::registry::AbiCtx). The
//! opaque `Process` handle is a descriptor-only nominal type (recognized by
//! [`is_builtin_type`]); the source companion carries only the `Stream`/`Signal`
//! value enums, injected as a helper-function chunk so the registry reassembles it.
//!
//! `process` is a fully data-only package: every call's return type is fixed per
//! name (the overloading is on argument shape, not return), and no overload uses
//! an argument *union*, so the registry's generic overload/return resolution
//! answers arity/return/validation with no custom resolver.

use crate::codegen::registry::{
    EnumVariant, Registry, RegistryEnum, RegistryPackage, RegistryResource,
};

mod func_close;
mod func_detach;

mod gen_shared;
mod gen_unix;
mod gen_windows;

// `process.__drop` is not a descriptor member (no `func_*.rs`), so its helper is
// still reached by name from the runtime-call dispatch; every other member lowers
// through its own per-member `Body::abi_function` body in its `func_*.rs`
// (`func_*.rs` → `gen_{unix,windows}`).
pub(crate) use gen_shared::lower_process_drop_helper;
mod func_did_signal;
mod func_is_running;
mod func_pid;
mod func_poll;
mod func_receive;
mod func_receive_bytes;
mod func_send;
mod func_send_bytes;
mod func_shell;
mod func_signal;
mod func_spawn;
mod func_wait_for;

/// The opaque `Process` resource handle's bare type name — its identity *within* the
/// `process` package (the `RegistryResource` name, the `type` half of the qualified
/// id). Used only for registry-internal lookups (`resolve_type`/close-op).
pub(crate) const PROCESS_TYPE: &str = "Process";

/// The `Process` resource's **package-qualified type identity** — how it flows through
/// the type system (bug-441 / plan-97: resources are addressed `process::Process`, not
/// bare `Process`, so a user `TYPE Process` no longer collides). This is the string a
/// `RES` binding of a spawned child carries, the `ResourceRegistry` key, and what
/// `is_resource`/close-op dispatch see. Dot form, matching the parser's internal
/// qualified spelling (`::` normalizes to `.`).
pub(crate) const PROCESS_TYPE_ID: &str = "process.Process";

/// The `Stream` enum (`StdOut`/`StdErr`) selecting which child pipe a read reads
/// from. Declared as an `EXPORT ENUM` in the source companion (plan-90-B).
pub(crate) const STREAM_TYPE: &str = "Stream";

/// The `Signal` enum (`None`/`Kill`/`Terminate`/`Error`) — the 4-bucket
/// send/observe vocabulary. Declared as an `EXPORT ENUM` in the companion
/// (plan-90-C).
pub(crate) const SIGNAL_TYPE: &str = "Signal";

/// The internal scope-drop op registered as `Process`'s resource close function.
///
/// Not user-callable: when a live `Process` goes out of scope the runtime
/// force-kills it (`SIGKILL`) and reaps it (`waitpid`) so no zombie is left and
/// drop never blocks. This is deliberately NOT the public `process::close`
/// (which closes only the child's stdin and leaves the child running) — so
/// `process::close(p)` is not treated as an ownership transfer and scope-drop
/// still runs `__drop`.
pub(crate) const DROP: &str = "process.__drop";

const MODULE_INTRO: &str =
    r#"Spawn and manage child processes: run a program, stream its standard I/O,"#;
const MODULE_DESC: &str = r#"The `process` package runs and controls child processes. Its one resource,
`Process`, is an opaque, owned, non-copyable handle to a spawned child — a native
resource sharing the runtime's canonical resource record (resource tag `10`).
Like every resource handle it cannot be copied, stored as a collection element,
or carried in a record; it is closed automatically by lexical drop when its
binding leaves scope.


A child is created two ways. `process::spawn` runs a program directly from an
argument list — `args[0]` is the executable, resolved on `PATH`, and no shell is
involved, so no quoting, globbing, or redirection is interpreted. `process::shell`
instead runs a command line through the platform shell (`/bin/sh -c` on Unix), so
pipes, redirection, and shell syntax work. A four-argument `spawn` overload adds a
working directory, an environment `Map OF String TO String`, and a replace-vs-merge
flag.


Ownership of a live child is deliberate. Letting a `Process` drop at scope exit
**force-kills and reaps** it (`SIGKILL` + `waitpid` on Unix), so no runaway child
or zombie is left behind and the drop never blocks. `process::close` is *not* a
handle-consuming close: it closes only the child's standard input (signalling
end-of-input to a filter) and leaves the child running and the handle usable.
`process::detach` relinquishes ownership the other way — it closes the parent-side
pipes, arranges for the child to be auto-reaped, and marks the handle closed so
the child keeps running independently after the program exits.



Streaming I/O connects to the child's three standard streams over pipes.
`process::send` writes a `String` (appending a newline) to the child's standard
input; `process::sendBytes` writes raw bytes with no newline. `process::receive`
reads one newline-terminated line as a `String`; `process::receiveBytes` reads one
available chunk of raw bytes. Both readers take an optional `Stream` argument
selecting standard output (the default) or standard error, and `process::poll`
reports whether the selected stream is readable within a timeout. A read that
reaches end of stream with nothing buffered raises `ErrResourceClosed`, so a
consumer loops until that error is raised.


The `Signal` enum is a four-bucket cross-platform vocabulary (`None`, `Kill`,
`Terminate`, `Error`) used both to *deliver* a signal with `process::signal` and
to *observe* how a terminated child died with `process::didSignal`; the exact
platform mapping is tabulated in `mfb man process types`.



The lifecycle queries read cached state: `process::pid` returns the child pid,
`process::isRunning` polls without blocking, `process::waitFor` blocks for exit
and returns the exit code (`-1` on a signal death on Unix). `waitFor` and
`isRunning` cache the exit status the first time they observe it, so `waitFor` is
idempotent and `didSignal` can report the death cause after the fact."#;

/// Register the `process` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("process", MODULE_INTRO, MODULE_DESC);

    // The two public value enums, modeled on the registry — `get_mfb` renders them
    // into the injected source in place of the hand-written `EXPORT ENUM`s that used to
    // be the package's whole source companion (`package.mfb` is now gone). `Stream`
    // (plan-90-B) selects which child stream a read reads from; `Signal` (plan-90-C) is
    // the cross-platform signal bucket.
    pkg.add_enum(RegistryEnum {
        name: STREAM_TYPE,
        export: true,
        variants: vec![
            EnumVariant {
                name: "StdOut",
                description: "The child's standard output.",
            },
            EnumVariant {
                name: "StdErr",
                description: "The child's standard error.",
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: SIGNAL_TYPE,
        export: true,
        variants: vec![
            EnumVariant {
                name: "None",
                description:
                    "No signal (a no-op to send; \"exited normally / still running\" to read).",
            },
            EnumVariant {
                name: "Kill",
                description: "Forced, uncatchable termination (SIGKILL).",
            },
            EnumVariant {
                name: "Terminate",
                description: "A polite \"please stop\" (SIGTERM and the other catchable stops).",
            },
            EnumVariant {
                name: "Error",
                description: "An abnormal-fault termination (SIGABRT/SIGSEGV/SIGFPE/…).",
            },
        ],
    });

    // The opaque `Process` resource handle. Semantic-only (no injectable source):
    // it makes `registry().qualified_builtin_type("process.Process")` and
    // `registry::resource_close_function("Process")` answer generically, replacing
    // the deleted per-package `is_builtin_type`/`resource_close_function` seams. The
    // `close_function` is the internal `__drop` scope-drop op (SIGKILL + waitpid); a
    // `Process` is released automatically by lexical scope, not a public `close`.
    pkg.add_resource(RegistryResource {
        name: PROCESS_TYPE,
        export: true,
        description: "An opaque handle to a spawned child process, released automatically \
                      when it leaves scope.",
        close_function: DROP,
        // A Process owns child pipe fds and drives waitpid from its owning thread; not
        // thread-sendable in v1 (plan-90-A; C revisits). The `__drop` op force-kills and
        // reaps, so it does not fail.
        sendable: false,
        close_may_fail: false,
        kind: crate::builtins::resource::ResourceKind::Builtin,
    });

    func_spawn::register(&mut pkg);
    func_shell::register(&mut pkg);
    func_pid::register(&mut pkg);
    func_is_running::register(&mut pkg);
    func_wait_for::register(&mut pkg);
    func_close::register(&mut pkg);
    func_send::register(&mut pkg);
    func_send_bytes::register(&mut pkg);
    func_receive::register(&mut pkg);
    func_receive_bytes::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_signal::register(&mut pkg);
    func_did_signal::register(&mut pkg);
    func_detach::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn process_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("process")
            .expect("process package");
        assert_eq!(pkg.functions().len(), 14);
        // The opaque handle is a semantic-only resource (not an EXPORT TYPE/UNION/
        // ENUM), so the value-type query `is_builtin_type` does not see it, but the
        // qualified-type resolution does (via the resource).
        assert!(!registry().is_builtin_type("Process"));
        assert_eq!(
            registry().qualified_builtin_type("process.Process"),
            Some("Process".to_string())
        );
    }

    #[test]
    fn generic_dispatch_reaches_process() {
        assert!(registry().is_member("process.spawn"));
        assert!(registry().is_member("process.didSignal"));
        assert!(!registry().is_member("process.nope"));
        // Native members carry no rewrite target (they lower through Body::abi_function).
        assert_eq!(registry::rewrite_target("process.spawn", &[]), None);
        // Fixed per-name return types.
        assert_eq!(registry::call_return_type("process.pid"), Some("Integer"));
        assert_eq!(
            registry::call_return_type("process.isRunning"),
            Some("Boolean")
        );
        assert_eq!(registry::call_return_type("process.close"), Some("Nothing"));
        assert_eq!(
            registry::call_return_type("process.spawn"),
            Some("process.Process")
        );
        assert_eq!(
            registry::call_return_type("process.receive"),
            Some("String")
        );
        assert_eq!(
            registry::call_return_type("process.didSignal"),
            Some("Signal")
        );
        // Arity ranges: spawn's two structurally distinct overloads (1 and 4 args),
        // the trailing-optional streaming forms, and the single-signature queries.
        assert_eq!(registry().arity("process.spawn"), Some((1, 4)));
        assert_eq!(registry().arity("process.shell"), Some((1, 1)));
        assert_eq!(registry().arity("process.pid"), Some((1, 1)));
        assert_eq!(registry().arity("process.send"), Some((2, 3)));
        assert_eq!(registry().arity("process.receive"), Some((1, 2)));
        assert_eq!(registry().arity("process.poll"), Some((2, 3)));
    }

    #[test]
    fn spawn_overloads_resolve() {
        let s = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            registry::resolve_call("process.spawn", &s(&["List OF String"]), false),
            Some("process.Process".to_string())
        );
        assert_eq!(
            registry::resolve_call(
                "process.spawn",
                &s(&[
                    "List OF String",
                    "String",
                    "Map OF String TO String",
                    "Boolean"
                ]),
                false
            ),
            Some("process.Process".to_string())
        );
        // The structural gap between the 1-arg and 4-arg overloads is rejected.
        assert_eq!(
            registry::resolve_call("process.spawn", &s(&[]), false),
            None
        );
        assert_eq!(
            registry::resolve_call("process.spawn", &s(&["List OF String", "String"]), false),
            None
        );
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("process")
            .expect("process")
            .get_mfb();
        // The reassembled companion is the `Stream`/`Signal` EXPORT ENUM source.
        assert!(source.contains("EXPORT ENUM Stream"));
        assert!(source.contains("EXPORT ENUM Signal"));
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-process>"),
            "builtins/process.mfb",
            &source,
        )
        .expect("reassembled process source parses");
    }

    #[test]
    fn process_close_op_is_drop() {
        assert_eq!(
            registry::resource_close_function(super::PROCESS_TYPE),
            Some(super::DROP)
        );
        assert_eq!(registry::resource_close_function("Nothing"), None);
    }

    #[test]
    fn spawn_expected_arguments_names_both_overloads() {
        // `spawn`'s overloaded phrasing rides on its descriptor field, served by the
        // generic `registry::expected_arguments`.
        let text =
            crate::codegen::registry::expected_arguments("process.spawn").expect("spawn phrasing");
        assert!(text.contains("List OF String"));
        assert!(text.contains(" or "));
        assert!(text.contains("Boolean"));
        // Single-signature calls render per-position from the registry.
        assert_eq!(
            crate::codegen::registry::expected_arguments("process.shell"),
            Some("String")
        );
    }

    #[test]
    fn runtime_call_membership() {
        // The runtime-call predicate is inlined at each dispatch site as
        // `owning_package(name) == Some("process") || name == process.__drop`.
        let is_runtime =
            |name: &str| registry().owning_package(name) == Some("process") || name == super::DROP;
        assert!(registry().owning_package("process.spawn") == Some("process"));
        assert!(is_runtime("process.spawn"));
        // `__drop` is the internal scope-drop op: a runtime call, not a descriptor call.
        assert!(registry().owning_package(super::DROP) != Some("process"));
        assert!(is_runtime(super::DROP));
        assert!(!is_runtime("process.bogus"));
    }
}
