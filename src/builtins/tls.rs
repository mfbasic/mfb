//! Front-end definitions for the built-in `tls` package (transport-layer
//! security, distinct from the thread-local-storage `tls` tokens elsewhere).
//!
//! `tls` is a native built-in like `net`: the Linux backend drives the system
//! OpenSSL (`libssl.so.3`, falling back to `libssl.so.1.1`) via `dlopen`/`dlsym`
//! so a single binary spans OpenSSL 1.1.1 and 3.x (plan-03-net.md §4.1). The
//! macOS backend drives Network.framework through a dispatch-semaphore
//! synchronous bridge.

use std::borrow::Cow;

pub(crate) const TLS_SOCKET_TYPE: &str = "TlsSocket";
pub(crate) const TLS_LISTENER_TYPE: &str = "TlsListener";

const CONNECT: &str = "tls.connect";
const LISTEN: &str = "tls.listen";
const ACCEPT: &str = "tls.accept";
const READ: &str = "tls.read";
const READ_TEXT: &str = "tls.readText";
const WRITE: &str = "tls.write";
const WRITE_TEXT: &str = "tls.writeText";
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

/// User-facing tls calls. Recognized by `is_builtin_call`, so it must NOT
/// include `CLOSE_LISTENER`, which is synthesized only during IR lowering and is
/// not user-callable (bug-173 E). A user-typed `tls.closeListener(x)` must be
/// reported as an unknown function.
pub(crate) fn is_tls_call(name: &str) -> bool {
    matches!(
        name,
        CONNECT | LISTEN | ACCEPT | READ | READ_TEXT | WRITE | WRITE_TEXT | CLOSE
    )
}

/// Post-lowering classifier: `is_tls_call` plus the internal listener-shaped
/// close body that IR lowering synthesizes. Used by codegen (`helper_for_call`,
/// per-target import planning) to route the lowered-only target.
pub(crate) fn is_tls_runtime_call(name: &str) -> bool {
    is_tls_call(name) || name == CLOSE_LISTENER
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == TLS_SOCKET_TYPE || name == TLS_LISTENER_TYPE
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
        CLOSE => Some(&[&["resource", "sock", "listener"]]),
        CLOSE_LISTENER => Some(&[&["listener"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        CONNECT => Some(TLS_SOCKET_TYPE),
        LISTEN => Some(TLS_LISTENER_TYPE),
        ACCEPT => Some(TLS_SOCKET_TYPE),
        READ => Some("List OF Byte"),
        READ_TEXT => Some("String"),
        WRITE | WRITE_TEXT | CLOSE | CLOSE_LISTENER => Some("Nothing"),
        _ => None,
    }
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
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        CONNECT => Some((2, 4)),
        LISTEN => Some((4, 5)),
        ACCEPT => Some((1, 2)),
        READ | READ_TEXT | WRITE | WRITE_TEXT => Some((2, 2)),
        CLOSE | CLOSE_LISTENER => Some((1, 1)),
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
    const CONNECT_DEFAULTS: &[(&str, &str)] = &[("Integer", "0"), ("String", "")];
    const LISTEN_DEFAULTS: &[(&str, &str)] = &[("Integer", "0")];
    const ACCEPT_DEFAULTS: &[(&str, &str)] = &[("Integer", "0")];
    match name {
        // connect(host, port, [timeoutMs=0], [serverName=""])
        CONNECT => &CONNECT_DEFAULTS[provided.saturating_sub(2).min(CONNECT_DEFAULTS.len())..],
        // listen(host, port, certPath, keyPath, [backlog=0]) — 0 uses the host
        // default backlog, mirroring net::listenTcp.
        LISTEN => &LISTEN_DEFAULTS[provided.saturating_sub(4).min(LISTEN_DEFAULTS.len())..],
        // accept(listener, [timeoutMs=0]) — 0 blocks without a deadline.
        ACCEPT => &ACCEPT_DEFAULTS[provided.saturating_sub(1).min(ACCEPT_DEFAULTS.len())..],
        _ => &[],
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `tls.close` consumes the handle it closes (either shape); `tls.accept`
/// only uses its listener (it stays open for the next accept).
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!((name, index), (CLOSE, 0) | (CLOSE_LISTENER, 0))
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}

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
            CONNECT, LISTEN, ACCEPT, READ, READ_TEXT, WRITE, WRITE_TEXT, CLOSE,
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
        // CONNECT is overloaded/defaulted -> None
        assert!(argument_types(CONNECT).is_none());
        assert!(argument_types("tls.nope").is_none());
    }

    #[test]
    fn arity_branches() {
        assert_eq!(arity(CONNECT), Some((2, 4)));
        assert_eq!(arity(LISTEN), Some((4, 5)));
        assert_eq!(arity(ACCEPT), Some((1, 2)));
        assert_eq!(arity(READ), Some((2, 2)));
        assert_eq!(arity(WRITE_TEXT), Some((2, 2)));
        assert_eq!(arity(CLOSE), Some((1, 1)));
        assert_eq!(arity(CLOSE_LISTENER), Some((1, 1)));
        assert!(arity("tls.nope").is_none());
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
}
