//! Front-end definitions for the built-in `tls` package (transport-layer
//! security, distinct from the thread-local-storage `tls` tokens elsewhere).
//!
//! `tls` is a native built-in like `net`: the Linux backend drives the system
//! OpenSSL (`libssl.so.3`, falling back to `libssl.so.1.1`) via `dlopen`/`dlsym`
//! so a single binary spans OpenSSL 1.1.1 and 3.x (plan-03-net.md §4.1). The
//! macOS backend drives Network.framework through a dispatch-semaphore
//! synchronous bridge.

use std::borrow::Cow;

use crate::codegen::registry::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinType, DefaultResolver,
    DefaultValue, Implementation, Lowering, Parameter, ParameterType, ReturnType, TypeKind,
};

pub(crate) const TLS_SOCKET_TYPE: &str = "TlsSocket";
pub(crate) const TLS_LISTENER_TYPE: &str = "TlsListener";

const CONNECT: &str = "tls.connect";
const LISTEN: &str = "tls.listen";
const ACCEPT: &str = "tls.accept";
const READ: &str = "tls.read";
const READ_TEXT: &str = "tls.readText";
const WRITE: &str = "tls.write";
const WRITE_TEXT: &str = "tls.writeText";
const POLL: &str = "tls.poll";
const CLOSE: &str = "tls.close";
/// Internal listener-shaped close body. `tls::close` stays the single
/// user-facing name over both handle types; IR lowering routes a `TlsListener`
/// operand here because the two records differ in shape (plan-06-tls-server.md
/// §4.1/§6.4). Not user-callable.
pub(crate) const CLOSE_LISTENER: &str = "tls.closeListener";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

// plan-72-Z: `TLS` is the descriptor authority for this package. tls is data-only
// (no resolver, like `net`): every call's return type is fixed per name (the
// `close` overloading is on the ARGUMENT type — `TlsSocket` vs `TlsListener` — not
// the return, which is always `Nothing`), so `call_return_type_name` and `arity`
// derive from the descriptor. Optional trailing arguments (`connect`'s
// `timeoutMs`/`serverName`, `listen`'s `backlog`, `accept`'s `timeoutMs`) are
// `DefaultValue::Fill`, so `DefaultResolver::default_padding` reproduces the legacy
// `default_argument_padding` (the genuinely custom padding the overview flagged is
// data-derivable — see Corrections in plan-72-Z). The two builtin types are opaque.
// `resolve_call` (`close(TlsSocket|TlsListener)` type-set acceptance),
// `expected_arguments` (`"or"`-phrased `close`), and `argument_types` (joined
// strings, overloaded → `None`) stay hand-authored. The lowered-only
// `CLOSE_LISTENER` is not a descriptor function; `call_return_type_name`/`arity`
// fall back to it explicitly (the `net`/`audio` internal-name pattern).
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn tf(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_intro: "",
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

// An optional trailing parameter padded during IR lowering — `Fill` with the same
// `(type, value)` the legacy `default_argument_padding` injects.
const fn fill(name: &'static str, ty: &'static str, expr: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr,
        },
    }
}

const SENTINEL: &str = crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL;

const P_CONNECT: &[Parameter] = &[
    req("host", &[], "String"),
    req("port", &[], "Integer"),
    fill("timeoutMs", "Integer", SENTINEL),
    fill("serverName", "String", ""),
];
const P_LISTEN: &[Parameter] = &[
    req("host", &[], "String"),
    req("port", &[], "Integer"),
    req("certPath", &[], "String"),
    req("keyPath", &[], "String"),
    fill("backlog", "Integer", "0"),
];
const P_ACCEPT: &[Parameter] = &[
    req("listener", &[], TLS_LISTENER_TYPE),
    fill("timeoutMs", "Integer", SENTINEL),
];
const P_READ: &[Parameter] = &[
    req("sock", &[], TLS_SOCKET_TYPE),
    req("maxBytes", &[], "Integer"),
];
const P_WRITE: &[Parameter] = &[
    req("sock", &[], TLS_SOCKET_TYPE),
    req("bytes", &[], "List OF Byte"),
];
const P_WRITE_TEXT: &[Parameter] = &[
    req("sock", &[], TLS_SOCKET_TYPE),
    req("value", &[], "String"),
];
// plan-76-B: TLS readiness query. An omitted `timeoutMs` pads the unbounded
// sentinel (block until readable), like `accept`. Readiness includes bytes already
// buffered in the TLS layer (decrypted, fd idle), not just raw-transport state.
const P_POLL: &[Parameter] = &[
    req("sock", &[], TLS_SOCKET_TYPE),
    fill("timeoutMs", "Integer", SENTINEL),
];
// plan-76-C: the TLS readiness multiplex. `socks` is a `List OF RES TlsSocket`; the
// returned `TlsSocket` is a BORROWED pointer to the first ready element (the list
// keeps ownership and closes each socket on scope exit), mirroring
// `net::poll(List OF RES Socket)`.
const P_POLL_LIST: &[Parameter] = &[
    req("socks", &[], "List OF RES TlsSocket"),
    fill("timeoutMs", "Integer", SENTINEL),
];
// `close` accepts either handle; the union is validated in the hand-authored
// `resolve_call`. The param's type feeds only the descriptor's `argument_types`/
// `expected_arguments` rendering, both of which tls keeps hand-authored, so the
// leading `TlsSocket` spelling (the `net::close` idiom) is inert here.
const P_CLOSE: &[Parameter] = &[req("resource", &["sock", "listener"], TLS_SOCKET_TYPE)];

const TLS_FUNCTIONS: &[BuiltinFunction] = &[
    tf(CONNECT, "connect", &[ov(P_CONNECT, TLS_SOCKET_TYPE)]),
    tf(LISTEN, "listen", &[ov(P_LISTEN, TLS_LISTENER_TYPE)]),
    tf(ACCEPT, "accept", &[ov(P_ACCEPT, TLS_SOCKET_TYPE)]),
    tf(READ, "read", &[ov(P_READ, "List OF Byte")]),
    tf(READ_TEXT, "readText", &[ov(P_READ, "String")]),
    tf(WRITE, "write", &[ov(P_WRITE, "Nothing")]),
    tf(WRITE_TEXT, "writeText", &[ov(P_WRITE_TEXT, "Nothing")]),
    tf(
        POLL,
        "poll",
        // Scalar readiness query (`TlsSocket → Boolean`) and the readiness multiplex
        // (`List OF RES TlsSocket → TlsSocket`, borrowed). Return types disagree, so
        // `resolve_call` selects by argument shape (plan-76-C).
        &[ov(P_POLL, "Boolean"), ov(P_POLL_LIST, TLS_SOCKET_TYPE)],
    ),
    tf(CLOSE, "close", &[ov(P_CLOSE, "Nothing")]),
];

const TLS_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: TLS_SOCKET_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: TLS_LISTENER_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
];

/// Return-type resolution for the tls calls, delegating to the hand-authored
/// `resolve_call` (which validates `close`'s `TlsSocket`/`TlsListener` argument
/// union that the descriptor's per-position match cannot). Exposed through the
/// descriptor so plan-72-BB can drive `tls::` return types from the registry.
/// Default padding is left to `DefaultResolver::default_padding` (the `Fill`
/// params reproduce it), so this resolver does not override it.
struct TlsResolver;
impl crate::codegen::registry::BuiltinResolver for TlsResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        resolve_call(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static TLS_RESOLVER: TlsResolver = TlsResolver;

pub(crate) static TLS: BuiltinModule = BuiltinModule {
    name: "tls",
    doc_intro: "",
    doc_desc: "",
    functions: TLS_FUNCTIONS,
    types: TLS_TYPES,
    source: None,
    resolver: Some(&TLS_RESOLVER),
};

/// User-facing tls calls. Recognized by `is_builtin_call`, so it must NOT
/// include `CLOSE_LISTENER`, which is synthesized only during IR lowering and is
/// not user-callable (bug-173 E). A user-typed `tls.closeListener(x)` must be
/// reported as an unknown function.
pub(crate) fn is_tls_call(name: &str) -> bool {
    DefaultResolver::contains(&TLS, name)
}

/// Post-lowering classifier: `is_tls_call` plus the internal listener-shaped
/// close body that IR lowering synthesizes. Used by codegen (`helper_for_call`,
/// per-target import planning) to route the lowered-only target.
pub(crate) fn is_tls_runtime_call(name: &str) -> bool {
    is_tls_call(name) || name == CLOSE_LISTENER
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    TLS.types.iter().any(|ty| ty.name == name)
}

/// plan-76-C: whether a `tls` call returns a BORROWED (non-owning) resource pointer
/// — `tls::poll(List OF RES TlsSocket)` returns a pointer to the first ready list
/// element; the list keeps ownership. Consulted by `value_aliases_live_resource`.
/// The scalar `tls::poll(TlsSocket)` returns `Boolean` and never binds a resource,
/// so keying on the call name alone is safe (as for `net::poll`).
pub(crate) fn returns_borrowed_resource(name: &str) -> bool {
    name == POLL
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        TLS_SOCKET_TYPE => Some(CLOSE),
        // Scope drops route straight to the listener-shaped internal close
        // body; the user-facing overload of `tls::close` over `TlsListener` is
        // rewritten to the same target during IR lowering.
        TLS_LISTENER_TYPE => Some(CLOSE_LISTENER),
        _ => None,
    }
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        CONNECT => Some(&[&["host"], &["port"], &["timeoutMs"], &["serverName"]]),
        LISTEN => Some(&[
            &["host"],
            &["port"],
            &["certPath"],
            &["keyPath"],
            &["backlog"],
        ]),
        ACCEPT => Some(&[&["listener"], &["timeoutMs"]]),
        READ | READ_TEXT => Some(&[&["sock"], &["maxBytes"]]),
        WRITE => Some(&[&["sock"], &["bytes"]]),
        WRITE_TEXT => Some(&[&["sock"], &["value"]]),
        // POLL is overloaded (scalar `sock` vs list `socks`); its per-overload names
        // live in `call_param_name_overloads` (plan-76-C).
        CLOSE => Some(&[&["resource", "sock", "listener"]]),
        CLOSE_LISTENER => Some(&[&["listener"]]),
        _ => None,
    }
}

/// plan-76-C: `tls::poll`'s two overloads name their receiver differently (`sock`
/// scalar, `socks` list), so the descriptor renders per-overload names here and
/// `call_param_names` yields `None` (mirrors `net::poll`).
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        POLL => Some(&[&["sock", "timeoutMs"], &["socks", "timeoutMs"]]),
        _ => None,
    }
}

/// Return type of a `tls::` call. Must keep the lowered-only `CLOSE_LISTENER`:
/// IR lowering rewrites a `TlsListener`-typed `tls::close` to `tls.closeListener`
/// and then queries this for the rewritten target's return type. User-facing
/// calls resolve through the descriptor; the internal name falls back explicitly
/// (the `net`/`audio` internal-name pattern).
pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    DefaultResolver::return_type_name(&TLS, name).or_else(|| match name {
        CLOSE_LISTENER => Some("Nothing"),
        _ => None,
    })
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        // connect(host, port, timeoutMs = 0, serverName = "")
        CONNECT
            if exact(arg_types, &["String", "Integer"])
                || exact(arg_types, &["String", "Integer", "Integer"])
                || exact(arg_types, &["String", "Integer", "Integer", "String"]) =>
        {
            Cow::Borrowed(TLS_SOCKET_TYPE)
        }
        // listen(host, port, certPath, keyPath, backlog = 0)
        LISTEN
            if exact(arg_types, &["String", "Integer", "String", "String"])
                || exact(
                    arg_types,
                    &["String", "Integer", "String", "String", "Integer"],
                ) =>
        {
            Cow::Borrowed(TLS_LISTENER_TYPE)
        }
        // accept(listener, timeoutMs = 0)
        ACCEPT
            if exact(arg_types, &[TLS_LISTENER_TYPE])
                || exact(arg_types, &[TLS_LISTENER_TYPE, "Integer"]) =>
        {
            Cow::Borrowed(TLS_SOCKET_TYPE)
        }
        READ if exact(arg_types, &[TLS_SOCKET_TYPE, "Integer"]) => Cow::Borrowed("List OF Byte"),
        READ_TEXT if exact(arg_types, &[TLS_SOCKET_TYPE, "Integer"]) => Cow::Borrowed("String"),
        WRITE if exact(arg_types, &[TLS_SOCKET_TYPE, "List OF Byte"]) => Cow::Borrowed("Nothing"),
        WRITE_TEXT if exact(arg_types, &[TLS_SOCKET_TYPE, "String"]) => Cow::Borrowed("Nothing"),
        // plan-76-B: readiness query `poll(TlsSocket[, timeoutMs]) → Boolean`.
        POLL if exact(arg_types, &[TLS_SOCKET_TYPE])
            || exact(arg_types, &[TLS_SOCKET_TYPE, "Integer"]) =>
        {
            Cow::Borrowed("Boolean")
        }
        // plan-76-C: readiness multiplex `poll(List OF RES TlsSocket[, timeoutMs]) →
        // TlsSocket` (borrowed element, like `collections::get`/`net::poll(List)`).
        POLL if exact(arg_types, &["List OF RES TlsSocket"])
            || exact(arg_types, &["List OF RES TlsSocket", "Integer"]) =>
        {
            Cow::Borrowed(TLS_SOCKET_TYPE)
        }
        CLOSE if exact(arg_types, &[TLS_SOCKET_TYPE]) || exact(arg_types, &[TLS_LISTENER_TYPE]) => {
            Cow::Borrowed("Nothing")
        }
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        CONNECT => Some("String, Integer, Integer, String"),
        LISTEN => Some("String, Integer, String, String, Integer"),
        ACCEPT => Some("TlsListener, Integer"),
        READ | READ_TEXT => Some("TlsSocket, Integer"),
        WRITE => Some("TlsSocket, List OF Byte"),
        WRITE_TEXT => Some("TlsSocket, String"),
        POLL => Some("TlsSocket, Integer or List OF RES TlsSocket, Integer"),
        CLOSE => Some("TlsSocket or TlsListener"),
        _ => None,
    }
}

/// Concrete per-argument types for literal coercion. Overloaded/defaulted calls
/// return `None` and rely on explicit argument types. `listen`/`accept` vary
/// only in trailing defaulted arity, so their positional types stay concrete.
pub(crate) fn argument_types(name: &str) -> Option<&'static str> {
    match name {
        LISTEN => Some("String, Integer, String, String, Integer"),
        ACCEPT => Some("TlsListener, Integer"),
        READ | READ_TEXT => Some("TlsSocket, Integer"),
        WRITE => Some("TlsSocket, List OF Byte"),
        WRITE_TEXT => Some("TlsSocket, String"),
        // POLL is overloaded → None (rely on explicit argument types), like net::poll.
        _ => None,
    }
}

/// Default trailing arguments to inject during IR lowering so the fixed-ABI
/// runtime helper always receives every parameter (plan-03-net.md §4). Returns
/// `(type, value)` constants to append after the `provided` real arguments.
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    // plan-73-D: an OMITTED `timeoutMs` pads the unbounded sentinel (i64::MIN) so
    // `tls::connect`/`tls::accept` block until the handshake completes (the timeout
    // convention's omit=unbounded rule); each backend routes the sentinel to its
    // block path, treats `0` as one immediate attempt, and rejects other negatives.
    const SENTINEL: &str = crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL;
    const CONNECT_DEFAULTS: &[(&str, &str)] = &[("Integer", SENTINEL), ("String", "")];
    const LISTEN_DEFAULTS: &[(&str, &str)] = &[("Integer", "0")];
    const ACCEPT_DEFAULTS: &[(&str, &str)] = &[("Integer", SENTINEL)];
    const POLL_DEFAULTS: &[(&str, &str)] = &[("Integer", SENTINEL)];
    match name {
        // connect(host, port, [timeoutMs=0], [serverName=""])
        CONNECT => &CONNECT_DEFAULTS[provided.saturating_sub(2).min(CONNECT_DEFAULTS.len())..],
        // listen(host, port, certPath, keyPath, [backlog=0]) — 0 uses the host
        // default backlog, mirroring net::listenTcp.
        LISTEN => &LISTEN_DEFAULTS[provided.saturating_sub(4).min(LISTEN_DEFAULTS.len())..],
        // accept(listener, [timeoutMs=0]) — 0 blocks without a deadline.
        ACCEPT => &ACCEPT_DEFAULTS[provided.saturating_sub(1).min(ACCEPT_DEFAULTS.len())..],
        // poll(sock, [timeoutMs]) — omitted blocks until readable (plan-76-B).
        POLL => &POLL_DEFAULTS[provided.saturating_sub(1).min(POLL_DEFAULTS.len())..],
        _ => &[],
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `tls.close` consumes the handle it closes (either shape); `tls.accept`
/// only uses its listener (it stays open for the next accept).
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!((name, index), (CLOSE, 0) | (CLOSE_LISTENER, 0))
}

use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn is_call_and_reject() {
        for n in [
            CONNECT, LISTEN, ACCEPT, READ, READ_TEXT, WRITE, WRITE_TEXT, POLL, CLOSE,
        ] {
            assert!(is_tls_call(n), "{n}");
            assert!(is_tls_runtime_call(n), "{n}");
        }
        // `closeListener` is lowered-only: recognized by the post-lowering
        // runtime classifier but NOT user-facing (bug-173 E).
        assert!(!is_tls_call(CLOSE_LISTENER));
        assert!(is_tls_runtime_call(CLOSE_LISTENER));
        assert!(!is_tls_call("tls.nope"));
        assert!(!is_tls_runtime_call("tls.nope"));
    }

    #[test]
    fn builtin_types_and_close_functions() {
        assert!(is_builtin_type(TLS_SOCKET_TYPE));
        assert!(is_builtin_type(TLS_LISTENER_TYPE));
        assert!(!is_builtin_type("String"));
        assert_eq!(resource_close_function(TLS_SOCKET_TYPE), Some(CLOSE));
        assert_eq!(
            resource_close_function(TLS_LISTENER_TYPE),
            Some(CLOSE_LISTENER)
        );
        assert_eq!(resource_close_function("Other"), None);
    }

    #[test]
    fn param_names_branches() {
        assert_eq!(call_param_names(CONNECT).unwrap().len(), 4);
        assert_eq!(call_param_names(LISTEN).unwrap().len(), 5);
        assert_eq!(call_param_names(ACCEPT).unwrap().len(), 2);
        assert_eq!(call_param_names(READ), call_param_names(READ_TEXT));
        assert_eq!(
            call_param_names(WRITE),
            Some(&[&["sock"][..], &["bytes"]][..])
        );
        assert_eq!(
            call_param_names(WRITE_TEXT),
            Some(&[&["sock"][..], &["value"]][..])
        );
        assert_eq!(
            call_param_names(CLOSE),
            Some(&[&["resource", "sock", "listener"][..]][..])
        );
        assert_eq!(
            call_param_names(CLOSE_LISTENER),
            Some(&[&["listener"][..]][..])
        );
        assert!(call_param_names("tls.nope").is_none());
    }

    #[test]
    fn return_type_name_branches() {
        assert_eq!(call_return_type_name(CONNECT), Some(TLS_SOCKET_TYPE));
        assert_eq!(call_return_type_name(LISTEN), Some(TLS_LISTENER_TYPE));
        assert_eq!(call_return_type_name(ACCEPT), Some(TLS_SOCKET_TYPE));
        assert_eq!(call_return_type_name(READ), Some("List OF Byte"));
        assert_eq!(call_return_type_name(READ_TEXT), Some("String"));
        assert_eq!(call_return_type_name(WRITE), Some("Nothing"));
        assert_eq!(call_return_type_name(WRITE_TEXT), Some("Nothing"));
        // POLL is return-type-overloaded (Boolean vs TlsSocket) → resolver-owned.
        assert_eq!(call_return_type_name(POLL), None);
        assert_eq!(call_return_type_name(CLOSE), Some("Nothing"));
        assert_eq!(call_return_type_name(CLOSE_LISTENER), Some("Nothing"));
        assert!(call_return_type_name("tls.nope").is_none());
    }

    #[test]
    fn resolve_connect_overloads() {
        assert_eq!(
            rt(CONNECT, &["String", "Integer"]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(
            rt(CONNECT, &["String", "Integer", "Integer"]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(
            rt(CONNECT, &["String", "Integer", "Integer", "String"]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(rt(CONNECT, &["String"]), None);
        assert_eq!(rt(CONNECT, &["Integer", "Integer"]), None);
    }

    #[test]
    fn resolve_listen_accept() {
        assert_eq!(
            rt(LISTEN, &["String", "Integer", "String", "String"]),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        assert_eq!(
            rt(
                LISTEN,
                &["String", "Integer", "String", "String", "Integer"]
            ),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        assert_eq!(rt(LISTEN, &["String", "Integer", "String"]), None);
        assert_eq!(
            rt(ACCEPT, &[TLS_LISTENER_TYPE]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(
            rt(ACCEPT, &[TLS_LISTENER_TYPE, "Integer"]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(rt(ACCEPT, &[TLS_SOCKET_TYPE]), None);
    }

    #[test]
    fn resolve_read_write_close() {
        assert_eq!(
            rt(READ, &[TLS_SOCKET_TYPE, "Integer"]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            rt(READ_TEXT, &[TLS_SOCKET_TYPE, "Integer"]),
            Some("String".to_string())
        );
        assert_eq!(
            rt(WRITE, &[TLS_SOCKET_TYPE, "List OF Byte"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            rt(WRITE_TEXT, &[TLS_SOCKET_TYPE, "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(rt(CLOSE, &[TLS_SOCKET_TYPE]), Some("Nothing".to_string()));
        assert_eq!(rt(CLOSE, &[TLS_LISTENER_TYPE]), Some("Nothing".to_string()));
        // plan-76-B: poll(TlsSocket[, Integer]) -> Boolean.
        assert_eq!(rt(POLL, &[TLS_SOCKET_TYPE]), Some("Boolean".to_string()));
        assert_eq!(
            rt(POLL, &[TLS_SOCKET_TYPE, "Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(rt(POLL, &["String"]), None);
        // plan-76-C: poll(List OF RES TlsSocket[, Integer]) -> TlsSocket (borrowed).
        assert_eq!(
            rt(POLL, &["List OF RES TlsSocket"]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(
            rt(POLL, &["List OF RES TlsSocket", "Integer"]),
            Some(TLS_SOCKET_TYPE.to_string())
        );
        assert_eq!(rt(READ, &[TLS_SOCKET_TYPE]), None);
        assert_eq!(rt(WRITE, &[TLS_SOCKET_TYPE, "String"]), None);
        assert_eq!(rt(CLOSE, &["String"]), None);
        assert_eq!(rt("tls.nope", &[]), None);
        // CLOSE_LISTENER is not user-callable through resolve_call
        assert_eq!(rt(CLOSE_LISTENER, &[TLS_LISTENER_TYPE]), None);
    }

    #[test]
    fn expected_arguments_branches() {
        assert_eq!(
            expected_arguments(CONNECT),
            Some("String, Integer, Integer, String")
        );
        assert_eq!(
            expected_arguments(LISTEN),
            Some("String, Integer, String, String, Integer")
        );
        assert_eq!(expected_arguments(ACCEPT), Some("TlsListener, Integer"));
        assert_eq!(expected_arguments(READ), Some("TlsSocket, Integer"));
        assert_eq!(expected_arguments(READ_TEXT), Some("TlsSocket, Integer"));
        assert_eq!(expected_arguments(WRITE), Some("TlsSocket, List OF Byte"));
        assert_eq!(expected_arguments(WRITE_TEXT), Some("TlsSocket, String"));
        assert_eq!(
            expected_arguments(POLL),
            Some("TlsSocket, Integer or List OF RES TlsSocket, Integer")
        );
        assert_eq!(expected_arguments(CLOSE), Some("TlsSocket or TlsListener"));
        assert!(expected_arguments(CLOSE_LISTENER).is_none());
        assert!(expected_arguments("tls.nope").is_none());
    }

    #[test]
    fn argument_types_branches() {
        assert_eq!(
            argument_types(LISTEN),
            Some("String, Integer, String, String, Integer")
        );
        assert_eq!(argument_types(ACCEPT), Some("TlsListener, Integer"));
        assert_eq!(argument_types(READ), Some("TlsSocket, Integer"));
        assert_eq!(argument_types(READ_TEXT), Some("TlsSocket, Integer"));
        assert_eq!(argument_types(WRITE), Some("TlsSocket, List OF Byte"));
        assert_eq!(argument_types(WRITE_TEXT), Some("TlsSocket, String"));
        // POLL is overloaded → None.
        assert_eq!(argument_types(POLL), None);
        // CONNECT is overloaded/defaulted -> None
        assert!(argument_types(CONNECT).is_none());
        assert!(argument_types("tls.nope").is_none());
    }

    #[test]
    fn default_padding_branches() {
        // connect(host, port, [timeoutMs=0], [serverName=""])
        assert_eq!(default_argument_padding(CONNECT, 2).len(), 2);
        assert_eq!(default_argument_padding(CONNECT, 3).len(), 1);
        assert_eq!(default_argument_padding(CONNECT, 4).len(), 0);
        assert_eq!(default_argument_padding(LISTEN, 4).len(), 1);
        assert_eq!(default_argument_padding(LISTEN, 5).len(), 0);
        assert_eq!(default_argument_padding(ACCEPT, 1).len(), 1);
        assert_eq!(default_argument_padding(ACCEPT, 2).len(), 0);
        // poll(sock, [timeoutMs]) — one defaulted trailing arg.
        assert_eq!(default_argument_padding(POLL, 1).len(), 1);
        assert_eq!(default_argument_padding(POLL, 2).len(), 0);
        assert_eq!(default_argument_padding(READ, 2), &[]);
    }

    #[test]
    fn consumes_argument_branches() {
        assert!(consumes_argument(CLOSE, 0));
        assert!(consumes_argument(CLOSE_LISTENER, 0));
        assert!(!consumes_argument(CLOSE, 1));
        assert!(!consumes_argument(ACCEPT, 0));
        assert!(!consumes_argument(WRITE, 0));
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov`/`tf`/`req`/`fill` are const fns used only in const context, so
        // their bodies never run at runtime. Call them at runtime to cover the
        // shape.
        let required = req("host", &["h"], "String");
        assert_eq!(required.name, "host");
        assert_eq!(required.aliases, &["h"]);
        assert_eq!(required.ty, ParameterType::Named("String"));
        assert_eq!(required.default, DefaultValue::None);

        let filled = fill("timeoutMs", "Integer", SENTINEL);
        assert_eq!(filled.name, "timeoutMs");
        assert!(filled.aliases.is_empty());
        assert_eq!(filled.ty, ParameterType::Named("Integer"));
        assert_eq!(
            filled.default,
            DefaultValue::Fill {
                type_name: "Integer",
                expr: SENTINEL,
            }
        );

        let overload = ov(P_READ, "List OF Byte");
        assert_eq!(overload.params.len(), 2);
        assert_eq!(overload.return_type, ReturnType::Fixed("List OF Byte"));

        // E0716: `tf` takes `&'static [BuiltinOverload]`, so build the slice as a
        // named const rather than a borrowed temporary.
        const OV: &[BuiltinOverload] = &[ov(P_READ, TLS_SOCKET_TYPE)];
        let func = tf(READ, "read", OV);
        assert_eq!(func.name, READ);
        assert_eq!(func.doc_slug, "read");
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Helper);
        assert_eq!(func.overloads.len(), 1);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }
}
