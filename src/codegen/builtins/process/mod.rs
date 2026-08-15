//! The built-in `process` package (plan-90).
//!
//! `process` spawns and manages child processes. Its one resource, `Process`, is
//! a native resource (tag 10) whose 96-byte record tail holds the three pipe fds
//! (stdin-write / stdout-read / stderr-read) plus the cached exit/signal state.
//!
//! `process` is the first **native** package migrated onto the clean-room registry
//! (`crate::codegen::registry`). Unlike the pure-MFBASIC packages already there
//! (`csv`/`json`/`regex`), every member owns a per-platform OS-seam lowering: its
//! `Body::Native` slots hold the `posix`/`win` runtime-helper emitters living in the
//! member's `func_*.rs` (delegating to the arch-neutral `native::{unix,windows}`
//! emission), which the runtime-call dispatch picks by `platform.family()`. The
//! opaque `Process` handle is a descriptor-only nominal type (recognized by
//! [`is_builtin_type`]); the source companion carries only the `Stream`/`Signal`
//! value enums, injected as a helper-function chunk so the registry reassembles it.
//!
//! `process` is a fully data-only package: every call's return type is fixed per
//! name (the overloading is on argument shape, not return), and no overload uses
//! an argument *union*, so the registry's generic overload/return resolution
//! answers arity/return/validation with no custom resolver.

use std::collections::HashMap;

use crate::codegen::registry::{Body, Registry, RegistryPackage};
use crate::target::shared::code::{CodegenPlatform, HelperResult, PlatformFamily};

mod func_close;
mod func_detach;

mod native;
pub(crate) mod specs;

// `process.__drop` is not a descriptor member (no `func_*.rs`), so its helper is
// still reached by name from the runtime-call dispatch; every other member lowers
// through its own `Body::Native` (`func_*.rs` → `native::{unix,windows}`).
pub(crate) use native::lower_process_drop_helper;
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

    // The source companion carries only the `Stream` (plan-90-B) and `Signal`
    // (plan-90-C) value enums; the opaque `Process` handle stays descriptor-only.
    // It imports nothing, so it has no source-ordering dependency.
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

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

/// Emit the `_mfb_rt_process_*` runtime-helper body for `call` from the owning
/// member's `Body::Native` lowering, chosen by `platform.family()`. `call` is the
/// member's own runtime-call name or one of the auxiliary code-form symbols the
/// `builder_values` overload split synthesizes (`process.spawnEnv`,
/// `process.sendTimeout`, `process.receiveFrom`, …); each aux form shares its
/// primary member's lowering fn, which branches on `call` internally. Returns
/// `None` for a `call` no `process` member owns.
pub(crate) fn dispatch_os_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Option<HelperResult> {
    // Route every runtime-call form (primary + aux) to the public member whose
    // `Body::Native` owns the emission — the migration's replacement for the old
    // `Implementation::Os { all }` covered-symbol list.
    let member = match call {
        "process.spawn" | "process.spawnEnv" => "process.spawn",
        "process.shell" => "process.shell",
        "process.pid" => "process.pid",
        "process.isRunning" => "process.isRunning",
        "process.waitFor" => "process.waitFor",
        "process.close" => "process.close",
        "process.send" | "process.sendTimeout" => "process.send",
        "process.sendBytes" | "process.sendBytesTimeout" => "process.sendBytes",
        "process.receive" | "process.receiveFrom" => "process.receive",
        "process.receiveBytes" | "process.receiveBytesFrom" => "process.receiveBytes",
        "process.poll" | "process.pollFrom" => "process.poll",
        "process.signal" => "process.signal",
        "process.didSignal" => "process.didSignal",
        "process.detach" => "process.detach",
        _ => return None,
    };
    let resolved = crate::codegen::registry::registry().resolve_func(member)?;
    let implementation = resolved.function.implementations().first()?;
    let Body::Native { posix, win, .. } = &implementation.body else {
        return None;
    };
    let lower = if platform.family() == PlatformFamily::Windows {
        (*win)?
    } else {
        (*posix)?
    };
    Some(lower(call, symbol, platform_imports, platform))
}

/// Whether `name` is a public `process` builtin call (`process.spawn`, …). The
/// internal `__drop` op and the `builder_values` aux code-form symbols are not
/// descriptor calls, so they are excluded here; use [`is_process_runtime_call`]
/// for the runtime-helper dispatch that includes `__drop`.
pub(crate) fn is_process_call(name: &str) -> bool {
    crate::codegen::registry::owning_package(name) == Some("process")
}

/// Whether `name` is a `process` call that lowers to a `_mfb_rt_process_*`
/// runtime helper — every public call plus the internal `__drop` cleanup op.
pub(crate) fn is_process_runtime_call(name: &str) -> bool {
    is_process_call(name) || name == DROP
}

/// A bespoke expected-argument phrasing for `spawn`, whose two overloads have
/// structurally different positional layouts. The registry's per-position render
/// only shows the FIRST overload (`List OF String`), which mis-describes a wrong
/// 4-arg call; this `"or"`-joined string names both forms (the net/audio idiom for
/// an overloaded call). Every other `process` call has a single signature the
/// registry renders correctly, so this returns `None` for them and they fall
/// through to the registry's `expected_arguments`.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        SPAWN => Some("List OF String or List OF String, String, Map OF String TO String, Boolean"),
        _ => None,
    }
}

/// Whether `name` is a `process` value/opaque type (`Process`). The `Stream`/
/// `Signal` value enums are recognized through the injected source companion, not
/// here; only the descriptor-only opaque handle is claimed.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == PROCESS_TYPE
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
    use crate::codegen::registry::{self, registry};

    #[test]
    fn process_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("process")
            .expect("process package");
        assert_eq!(pkg.functions().len(), 14);
        // The opaque handle is descriptor-only (not an EXPORT TYPE/UNION), so the
        // generic type query does not see it; the hand-written predicate does.
        assert!(super::is_builtin_type("Process"));
        assert!(!registry::is_builtin_type("Process"));
    }

    #[test]
    fn generic_dispatch_reaches_process() {
        assert!(registry::is_member("process.spawn"));
        assert!(registry::is_member("process.didSignal"));
        assert!(!registry::is_member("process.nope"));
        // Native members carry no rewrite target (they lower through Body::Native).
        assert_eq!(registry::rewrite_target("process.spawn", &[]), None);
        // Fixed per-name return types.
        assert_eq!(registry::call_return_type("process.pid"), Some("Integer"));
        assert_eq!(
            registry::call_return_type("process.isRunning"),
            Some("Boolean")
        );
        assert_eq!(registry::call_return_type("process.close"), Some("Nothing"));
        assert_eq!(registry::call_return_type("process.spawn"), Some("Process"));
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
        assert_eq!(registry::arity("process.spawn"), Some((1, 4)));
        assert_eq!(registry::arity("process.shell"), Some((1, 1)));
        assert_eq!(registry::arity("process.pid"), Some((1, 1)));
        assert_eq!(registry::arity("process.send"), Some((2, 3)));
        assert_eq!(registry::arity("process.receive"), Some((1, 2)));
        assert_eq!(registry::arity("process.poll"), Some((2, 3)));
    }

    #[test]
    fn spawn_overloads_resolve() {
        let s = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            registry::resolve_call("process.spawn", &s(&["List OF String"])),
            Some("Process".to_string())
        );
        assert_eq!(
            registry::resolve_call(
                "process.spawn",
                &s(&[
                    "List OF String",
                    "String",
                    "Map OF String TO String",
                    "Boolean"
                ])
            ),
            Some("Process".to_string())
        );
        // The structural gap between the 1-arg and 4-arg overloads is rejected.
        assert_eq!(registry::resolve_call("process.spawn", &s(&[])), None);
        assert_eq!(
            registry::resolve_call("process.spawn", &s(&["List OF String", "String"])),
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
            super::resource_close_function(super::PROCESS_TYPE),
            Some(super::DROP)
        );
        assert_eq!(super::resource_close_function("Nothing"), None);
    }

    #[test]
    fn spawn_expected_arguments_names_both_overloads() {
        let text = super::expected_arguments("process.spawn").expect("spawn phrasing");
        assert!(text.contains("List OF String"));
        assert!(text.contains(" or "));
        assert!(text.contains("Boolean"));
        // Single-signature calls fall through to the registry renderer.
        assert_eq!(super::expected_arguments("process.shell"), None);
        assert_eq!(super::expected_arguments("process.close"), None);
    }

    #[test]
    fn runtime_call_membership() {
        assert!(super::is_process_call("process.spawn"));
        assert!(super::is_process_runtime_call("process.spawn"));
        // `__drop` is the internal scope-drop op: a runtime call, not a descriptor call.
        assert!(!super::is_process_call(super::DROP));
        assert!(super::is_process_runtime_call(super::DROP));
        assert!(!super::is_process_call("process.bogus"));
        assert!(!super::is_process_runtime_call("process.bogus"));
    }
}
