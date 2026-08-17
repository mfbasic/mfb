//! The built-in `thread` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`).
//!
//! `thread` is a **descriptor-only** migration. Unlike `process`/`fs`/`io`, thread's
//! runtime lowering is NOT a per-member OS-seam: every user call lowers through the
//! shared code-layer `RuntimeHelper::Thread` machinery (the per-thread data/resource
//! queues, the parent/worker direction split in `builder_values`, the cancellation
//! condvars, and the `_mfb_rt_stdin_*` broadcast) by its call NAME. So each member's
//! [`Body`] is [`Body::Intrinsic`] — no rewrite, no source body — and the call name is
//! left intact for `runtime::helper_for_call` to route. The runtime ABI catalog
//! (`thread_specs.rs`), the `thread.drop` scope-cleanup op, and the stdin broadcast
//! all stay shared (plan §"PART B obstacle resolution"); only the DESCRIPTOR
//! (membership, arity, parameter names, argument-typed return resolution, the opaque
//! `Thread`/`ThreadWorker` handle types) moves here off the legacy
//! `target::shared::registry` `THREAD` module + `ThreadResolver`.
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
fn th(worker: bool, msg: ParameterType, res: ParameterType, out: ParameterType) -> ParameterType {
    ParameterType::thread_handle(worker, msg, res, out)
}

// A required parameter.
fn req(name: &'static str, aliases: &'static [&'static str], ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

// An optional trailing parameter (widens arity; not default-padded).
fn opt(name: &'static str, aliases: &'static [&'static str], ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases,
        ty,
        default: DefaultValue::Optional,
    }
}

// A single Body::Intrinsic overload: params + return type. thread lowers by call
// name through the shared `RuntimeHelper::Thread`, so no rewrite/source body.
fn overload(params: Vec<Parameter>, return_type: ParameterType) -> Implementation {
    Implementation {
        params,
        return_type,
        errors: vec![],
        body: Body::Intrinsic,
    }
}

fn function(
    name: &'static str,
    expected_arguments: Option<&'static str>,
    implementations: Vec<Implementation>,
) -> RegistryFunction {
    RegistryFunction {
        name,
        intro: "",
        desc: "",
        example: "",
        expected_arguments,
        internal_only: false,
        implementations,
    }
}

/// Register the `thread` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("thread", "", "");

    // The two opaque handle type NAMES. Recorded as source-declared (opaque) types so
    // the generic `registry::is_builtin_type`/`qualified_builtin_type` recognize them
    // (bare and, via the parametric extension, `Thread OF … TO …`) without an
    // injectable source and without the RES resource-table machinery — a `Thread` is
    // cleaned by `builder_thread_cleanup` + the shared `thread.drop` op, not a close op.
    pkg.add_source_types(&[THREAD_TYPE, THREAD_WORKER_TYPE]);

    let u = || ParameterType::Unknown;
    let msg = || ParameterType::Var("Msg");
    let out = || ParameterType::Var("Out");
    let res = || ParameterType::Var("Res");
    let nothing = || ParameterType::Nothing;
    // A handle that echoes NO slot — a wildcard in every position (accepts any
    // msg/res/out including `Nothing`).
    let any = |worker: bool| th(worker, u(), u(), u());

    // start: the worker's msg/res/out are echoed onto the returned parent handle, and
    // any of msg/res can be `Nothing` (a resource-only or data-less worker). Since a
    // `Var` cannot bind `Nothing` under strict validation, each `Nothing` case is a
    // distinct overload — the msg × res `{Var, Nothing}` matrix (out is always the
    // worker's return, a real value). The all-`Var` overload is FIRST so lenient
    // return-inference binds every slot (a `Nothing` slot binds under lenient and
    // elides in `name()`); the `Nothing`-literal overloads exist for strict validation.
    let start_overload = |worker_msg: ParameterType, worker_res: ParameterType| {
        overload(
            vec![
                req(
                    "f",
                    &["entry"],
                    ParameterType::func_isolated(
                        vec![
                            th(true, worker_msg.clone(), worker_res.clone(), out()),
                            ParameterType::Var("In"),
                        ],
                        out(),
                    ),
                ),
                req("data", &[], ParameterType::Var("In")),
                opt("inboundLimit", &[], ParameterType::Integer),
                opt("outboundLimit", &[], ParameterType::Integer),
            ],
            th(false, worker_msg, worker_res, out()),
        )
    };
    pkg.add_function(function(
        "start",
        Some("ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, In, Integer, Integer"),
        vec![
            start_overload(msg(), res()),
            start_overload(msg(), nothing()),
            start_overload(nothing(), res()),
            start_overload(nothing(), nothing()),
        ],
    ));

    // Parent-only queries — echo no slot.
    pkg.add_function(function(
        "isRunning",
        Some("Thread OF Msg TO Out"),
        vec![overload(
            vec![req("t", &["thread"], any(false))],
            ParameterType::Boolean,
        )],
    ));
    // waitFor echoes the output. A single `Var`-output overload: a wholly-`Unknown`
    // (not-yet-inferred) handle leaves `Out` unbound so the return is `None`
    // (retryable) rather than a spurious concrete — a `Nothing`-return overload would
    // wildcard-match an `Unknown` handle and poison inference to `Nothing`.
    pkg.add_function(function(
        "waitFor",
        Some("Thread OF Msg TO Out"),
        vec![overload(
            vec![req("t", &["thread"], th(false, u(), u(), out()))],
            out(),
        )],
    ));
    pkg.add_function(function(
        "cancel",
        Some("Thread OF Msg TO Out"),
        vec![overload(
            vec![req("t", &["thread"], any(false))],
            ParameterType::Nothing,
        )],
    ));
    pkg.add_function(function(
        "poll",
        Some("Thread OF Msg TO Out, Integer"),
        vec![overload(
            vec![
                req("t", &["thread"], any(false)),
                req("ms", &[], ParameterType::Integer),
            ],
            ParameterType::Boolean,
        )],
    ));

    // Worker-only query.
    pkg.add_function(function(
        "isCancelled",
        Some("ThreadWorker OF Msg TO Out"),
        vec![overload(
            vec![req("t", &["thread"], any(true))],
            ParameterType::Boolean,
        )],
    ));

    // send constrains arg1 to the handle's message type (two kind-split overloads).
    pkg.add_function(function(
        "send",
        Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Msg, Integer"),
        vec![
            overload(send_params(false, msg()), ParameterType::Nothing),
            overload(send_params(true, msg()), ParameterType::Nothing),
        ],
    ));
    // receive echoes the message (two kind-split overloads). Like waitFor, no
    // `Nothing`-return overload — that would wildcard-match an `Unknown` handle.
    pkg.add_function(function(
        "receive",
        Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Integer"),
        vec![
            overload(receive_params(false, msg()), msg()),
            overload(receive_params(true, msg()), msg()),
        ],
    ));
    pkg.add_function(function(
        "sleep",
        Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Integer"),
        vec![
            overload(sleep_params(false), ParameterType::Nothing),
            overload(sleep_params(true), ParameterType::Nothing),
        ],
    ));

    // Either-kind resource-plane members → two kind-split, resource-ONLY overloads.
    // The handle's msg/out are wildcards (a resource plane rides any data plane); only
    // `res` is captured, so a data-only handle (`res: Nothing`) is rejected by strict.
    pkg.add_function(function(
        "transfer",
        Some("Thread OF Msg RES Res TO Out or ThreadWorker OF Msg RES Res TO Out, Res, Integer"),
        vec![
            overload(transfer_params(false), ParameterType::Nothing),
            overload(transfer_params(true), ParameterType::Nothing),
        ],
    ));
    pkg.add_function(function(
        "accept",
        Some("Thread OF Msg RES Res TO Out or ThreadWorker OF Msg RES Res TO Out, Integer"),
        vec![
            overload(accept_params(false), res()),
            overload(accept_params(true), res()),
        ],
    ));

    // stdin broadcast: zero args (calling thread) OR one parent handle (that worker).
    pkg.add_function(function(
        "openStdIn",
        Some("() or Thread OF Msg TO Out"),
        vec![overload(
            vec![opt("t", &["thread"], any(false))],
            ParameterType::Nothing,
        )],
    ));
    pkg.add_function(function(
        "closeStdIn",
        Some("() or Thread OF Msg TO Out"),
        vec![overload(
            vec![opt("t", &["thread"], any(false))],
            ParameterType::Nothing,
        )],
    ));

    r.add_package(pkg);
}

fn send_params(worker: bool, msg: ParameterType) -> Vec<Parameter> {
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
        ),
        req("data", &["value"], msg),
        opt("timeoutMs", &[], ParameterType::Integer),
    ]
}

fn receive_params(worker: bool, msg: ParameterType) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(worker, msg, ParameterType::Unknown, ParameterType::Unknown),
        ),
        opt("timeoutMs", &[], ParameterType::Integer),
    ]
}

fn sleep_params(worker: bool) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(
                worker,
                ParameterType::Unknown,
                ParameterType::Unknown,
                ParameterType::Unknown,
            ),
        ),
        req("ms", &[], ParameterType::Integer),
    ]
}

fn transfer_params(worker: bool) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(
                worker,
                ParameterType::Unknown,
                ParameterType::Var("Res"),
                ParameterType::Unknown,
            ),
        ),
        req("res", &["resource"], ParameterType::Var("Res")),
        opt("timeoutMs", &[], ParameterType::Integer),
    ]
}

fn accept_params(worker: bool) -> Vec<Parameter> {
    vec![
        req(
            "t",
            &["thread"],
            th(
                worker,
                ParameterType::Unknown,
                ParameterType::Var("Res"),
                ParameterType::Unknown,
            ),
        ),
        opt("timeoutMs", &[], ParameterType::Integer),
    ]
}

#[cfg(test)]
mod tests;
