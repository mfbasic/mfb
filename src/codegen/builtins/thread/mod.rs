//! The built-in `thread` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`).
//!
//! Each member owns its `Body::abi_function` body in [`lowering`] (`lower_<name>`),
//! the clean-room shape shared with every other builtin. The heavy per-thread machinery
//! (data/resource queues, cancellation condvars, the `_mfb_rt_stdin_*` broadcast) stays
//! in the shared `codegen::runtime::thread` emitters (`simple_thread_handle_helper`,
//! `thread_queue_write_helper`/`thread_queue_read_helper`, `lower_thread_start_helper`,
//! …); each member body is a thin call to its emitter that returns un-finalized parts,
//! and the `abi_function` wrapper seeds `entry` and finalizes. The parent/worker
//! direction split + the resource plane resolve to distinct runtime-call NAMES in
//! `builder_values` (`send`→`emit`, `receive`→`read`, `transfer`
//! →`transferResource`/`emitResource`, `accept`→`acceptResource`/`readResource`); each
//! such code form is an `os_alias` of the owning member, so `abi_function_lower` routes
//! it to that member's body, which reads [`AbiCtx::call`](crate::codegen::registry::AbiCtx)
//! to pick the queue/direction. The `thread.drop` scope-cleanup op is an `os_alias` of
//! `cancel`. `thread.start` reads `AbiCtx::arena_global_slots`/`uses_rng` (the sole
//! per-compilation build state a thread body consumes). The runtime ABI catalog is now
//! DERIVED from the registry (`registry::runtime_specs`) — no hand-written thread specs.
//! The DESCRIPTOR (membership, arity, parameter names, argument-typed return resolution,
//! the opaque `Thread`/`ThreadWorker` handle types) lives here.
//!
//! The argument-typed return resolution — `start` builds a fresh parent handle,
//! `waitFor`→Out, `receive`→Msg, `accept`→Res, `send`/`transfer` cross-param — rides
//! the generic [`ParameterType::ThreadHandle`] unify/substitute with no custom
//! resolver. The resource plane is a SIGNATURE-LEVEL overload split on the existing
//! `RES` spelling: `start` has a resource overload (tried first) plus a data overload;
//! `accept`/`transfer` are resource-only, so a data-only handle is rejected by the
//! strict-`Nothing` guard exactly as the legacy resolver returned `None`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, Registry, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

mod lowering;

// One file per member, holding that member's descriptor AND its man-page prose —
// the same shape every other builtin package uses. The shared signature helpers
// (`th`, `req`, `opt`, `overload`, `function`, and the `*_params` builders) stay
// here because several members spell the same parameter shape.
mod func_accept;
mod func_cancel;
mod func_close_std_in;
mod func_is_cancelled;
mod func_is_running;
mod func_open_std_in;
mod func_poll;
mod func_receive;
mod func_send;
mod func_start;
mod func_transfer;
mod func_wait_for;

const MODULE_INTRO: &str =
    "Run an isolated function on its own thread, and talk to it while it runs";

const MODULE_DESC: &str = r#"The `thread` package starts a function running on its own thread, sends messages
back and forth while it runs, hands open resources across, and collects the
result when it finishes.

A thread runs an `ISOLATED FUNC` — a function declared so that it shares nothing
with whoever started it. It gets its own copy of its package's top-level state,
so two threads running the same function never see each other's variables. The
entry point takes the worker's own handle as its first argument and one value of
your choosing as its second:

```
EXPORT ISOLATED FUNC parseFile(worker AS ThreadWorker OF String TO Integer, path AS String) AS Integer
```

`thread::start` launches it and gives you a `Thread` handle; the function itself
receives the matching `ThreadWorker` handle. Both name the same running thread
from its two ends, and most calls here take either one — the parent side passes
its `Thread`, the worker side passes its `ThreadWorker`.

Everything that crosses the boundary is copied, so no value is ever reachable
from two threads at once. Values allowed to cross are called thread-sendable:
numbers, `String`, records, unions and immutable containers are, as long as
everything inside them is. Functions and lambdas are not.

There are two separate channels, and one thread may use both at once. The
**message channel** carries data — `thread::send`, `thread::receive` and
`thread::poll`, typed by the `Msg` in `Thread OF Msg TO Out`. The **resource
channel** carries open handles — `thread::transfer` and `thread::accept`, typed
by the `Res` in `Thread OF Msg RES Res TO Out`. A resource may not travel on the
message channel; declare it on the resource channel instead. Not every resource
type may cross: among the built-in ones `fs::File`, `tcp::Socket` and
`udp::Socket` may, while listeners and `tls::Socket` may not. A resource your
own project declares may cross when it is declared `THREAD_SENDABLE`.

Both channels are queues with a size limit, set by `thread::start`'s
`inboundLimit` and `outboundLimit` and defaulting to 64 messages each. When a
queue is full the sender waits, so a fast producer cannot outrun a slow
consumer.

`thread::waitFor` waits for the thread to finish and gives you its return value
— or, if the function failed, fails the same way in your own code, carrying the
same error. It is also the end of the handle: `waitFor` closes it, and using
that handle again raises `ErrResourceClosed`.

`thread::cancel` asks a thread to stop. It is a request, not a kill: the worker
notices by calling `thread::isCancelled` and decides what to do about it. A
thread that never asks runs to completion.

`thread::openStdIn` and `thread::closeStdIn` let more than one thread read
standard input. Every subscriber gets its own view of the stream, so a line read
by one thread is never taken away from another.

`thread` is a built-in package: `IMPORT thread` needs no manifest dependency."#;

/// The parent thread handle's opaque type name.
pub(crate) const THREAD_TYPE: &str = crate::types::THREAD_TYPE;
/// The worker thread handle's opaque type name.
pub(crate) const THREAD_WORKER_TYPE: &str = crate::types::THREAD_WORKER_TYPE;

/// Internal lowered targets for the resource plane — never user-callable. A source
/// `thread::transfer`/`thread::accept` is rewritten to these during IR lowering
/// (`thread_resource_plane_target`), then split by handle direction in
/// `builder_values` (`transferResource`↔`emitResource`, `acceptResource`↔
/// `readResource`) so codegen routes them to the dedicated per-thread resource
/// queues. The runtime ABI catalog (`thread_specs.rs`) and the queue lowering stay
/// shared; only the descriptor moved onto the registry.
pub(crate) const TRANSFER_RESOURCE: &str = "thread.transferResource";
pub(crate) const ACCEPT_RESOURCE: &str = "thread.acceptResource";
pub(crate) const EMIT_RESOURCE: &str = "thread.emitResource";
pub(crate) const READ_RESOURCE: &str = "thread.readResource";

/// Whether `name` is a user-facing `thread::` call (a registered descriptor member).
/// The registry-backed replacement for the legacy `DefaultResolver::contains(&THREAD, …)`
/// — it must NOT include the internal resource-plane names, which are synthesized
/// only during IR lowering and are not user-callable (bug-173 E).
pub(crate) fn is_thread_call(name: &str) -> bool {
    crate::codegen::registry::registry().owning_package(name) == Some("thread")
}

/// Post-lowering classifier: [`is_thread_call`] plus the internal resource-plane
/// names that IR lowering synthesizes. Used by `runtime::helper_for_call` to route
/// codegen for these lowered-only targets. Byte-identical to the legacy predicate.
pub(crate) fn is_thread_runtime_call(name: &str) -> bool {
    is_thread_call(name)
        || matches!(
            name,
            TRANSFER_RESOURCE | ACCEPT_RESOURCE | EMIT_RESOURCE | READ_RESOURCE
        )
}

// A `Thread`/`ThreadWorker` handle pattern with explicit per-slot patterns. A slot a
// member does not echo is [`Unknown`](ParameterType::Unknown) — a wildcard that
// accepts any concrete slot INCLUDING `Nothing` (a resource-only thread's message, a
// `Nothing`-returning worker's output), so the strict-`Nothing` guard (which rejects a
// `Var` bound to `Nothing`) only bites where a member genuinely captures a slot. A slot
// a member ECHOES uses `Var(..)`; a `Nothing`-valued echoed slot is handled by an
// explicit `Nothing`-literal overload (the signature-level split, e.g. `start`).
pub(super) fn th(
    worker: bool,
    msg: ParameterType,
    res: ParameterType,
    out: ParameterType,
) -> ParameterType {
    ParameterType::thread_handle(worker, msg, res, out)
}

// A handle that echoes NO slot — a wildcard in every position (accepts any
// msg/res/out including `Nothing`).
pub(super) fn any(worker: bool) -> ParameterType {
    th(
        worker,
        ParameterType::Unknown,
        ParameterType::Unknown,
        ParameterType::Unknown,
    )
}

// A required parameter. `desc` is the man page's Parameters-table prose.
pub(super) fn req(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
    desc: &'static str,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

// An optional trailing parameter (widens arity; not default-padded).
pub(super) fn opt(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
    desc: &'static str,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases,
        ty,
        default: DefaultValue::Optional,
    }
}

// A single overload: params + return type + the member's per-member `abi_function`
// body. Every overload of a member shares one body (`lowering::lower_<name>`), which
// branches its worker/parent + resource-plane split off `AbiCtx::call`.
pub(super) fn overload(
    params: Vec<Parameter>,
    return_type: ParameterType,
    body: Body,
) -> Implementation {
    Implementation {
        params,
        return_type,
        errors: vec![],
        body,
    }
}

// `prose` is (intro, desc, example) — the three man-page fields.
pub(super) fn function(
    name: &'static str,
    expected_arguments: Option<&'static str>,
    prose: (&'static str, &'static str, &'static str),
    implementations: Vec<Implementation>,
) -> RegistryFunction {
    let (intro, desc, example) = prose;
    RegistryFunction {
        name,
        intro,
        desc,
        example,
        expected_arguments,
        internal_only: false,
        implementations,
    }
}

/// The message-plane parameter shape shared by `send`'s two kind-split overloads.
pub(super) fn send_params(worker: bool, msg: ParameterType) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(
                worker,
                msg.clone(),
                ParameterType::Unknown,
                ParameterType::Unknown,
            ),
            "The thread to send to — your `Thread` handle from the parent side, or the worker's own `ThreadWorker` handle from inside the thread.",
        ),
        req(
            "data",
            &["value"],
            msg,
            "The value to send. It must match the handle's message type and be thread-sendable; it is copied, so the two sides never share it.",
        ),
        opt(
            "timeoutMs",
            &[],
            ParameterType::Integer,
            "How long to wait, in milliseconds, if the queue is already full.",
        ),
    ]
}

/// The message-plane parameter shape shared by `receive`'s two kind-split overloads.
pub(super) fn receive_params(worker: bool, msg: ParameterType) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(worker, msg, ParameterType::Unknown, ParameterType::Unknown),
            "The thread to read from — your `Thread` handle from the parent side, or the worker's own `ThreadWorker` handle from inside the thread.",
        ),
        opt(
            "timeoutMs",
            &[],
            ParameterType::Integer,
            "How long to wait, in milliseconds, for a message to arrive.",
        ),
    ]
}

/// The resource-plane parameter shape shared by `transfer`'s two overloads.
pub(super) fn transfer_params(worker: bool) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(
                worker,
                ParameterType::Unknown,
                ParameterType::var("Res"),
                ParameterType::Unknown,
            ),
            "The thread to hand the resource to — your `Thread` handle from the parent side, or the worker's own `ThreadWorker` handle from inside the thread.",
        ),
        req(
            "res",
            &["resource"],
            ParameterType::var("Res"),
            "The open handle to hand over. It must match the thread's resource type and be one of the types allowed to cross. This call takes the handle: on success you cannot use it again.",
        ),
        opt(
            "timeoutMs",
            &[],
            ParameterType::Integer,
            "How long to wait, in milliseconds, if the resource queue is already full.",
        ),
    ]
}

/// The resource-plane parameter shape shared by `accept`'s two overloads.
pub(super) fn accept_params(worker: bool) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(
                worker,
                ParameterType::Unknown,
                ParameterType::var("Res"),
                ParameterType::Unknown,
            ),
            "The thread to take the resource from — your `Thread` handle from the parent side, or the worker's own `ThreadWorker` handle from inside the thread.",
        ),
        opt(
            "timeoutMs",
            &[],
            ParameterType::Integer,
            "How long to wait, in milliseconds, for a resource to arrive.",
        ),
    ]
}

/// Register the `thread` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("thread", MODULE_INTRO, MODULE_DESC);

    // The two opaque handle type NAMES. Recorded as source-declared (opaque) types so
    // the generic `registry::is_builtin_type`/`qualified_builtin_type` recognize them
    // (bare and, via the parametric extension, `Thread OF … TO …`) without an
    // injectable source and without the RES resource-table machinery — a `Thread` is
    // cleaned by `builder_thread_cleanup` + the shared `thread.drop` op, not a close op.
    pkg.add_source_types(&[THREAD_TYPE, THREAD_WORKER_TYPE]);

    func_start::register(&mut pkg);
    func_is_running::register(&mut pkg);
    func_wait_for::register(&mut pkg);
    func_cancel::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_is_cancelled::register(&mut pkg);
    func_send::register(&mut pkg);
    func_receive::register(&mut pkg);
    func_transfer::register(&mut pkg);
    func_accept::register(&mut pkg);
    func_open_std_in::register(&mut pkg);
    func_close_std_in::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests;
