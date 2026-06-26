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

const CONNECT: &str = "tls.connect";
const READ: &str = "tls.read";
const READ_TEXT: &str = "tls.readText";
const WRITE: &str = "tls.write";
const WRITE_TEXT: &str = "tls.writeText";
const CLOSE: &str = "tls.close";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_tls_call(name: &str) -> bool {
    matches!(
        name,
        CONNECT | READ | READ_TEXT | WRITE | WRITE_TEXT | CLOSE
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == TLS_SOCKET_TYPE
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    (type_name == TLS_SOCKET_TYPE).then_some(CLOSE)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        CONNECT => Some(&[&["host"], &["port"], &["timeoutMs"], &["serverName"]]),
        READ | READ_TEXT => Some(&[&["sock"], &["maxBytes"]]),
        WRITE => Some(&[&["sock"], &["bytes"]]),
        WRITE_TEXT => Some(&[&["sock"], &["value"]]),
        CLOSE => Some(&[&["resource", "sock"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        CONNECT => Some(TLS_SOCKET_TYPE),
        READ => Some("List OF Byte"),
        READ_TEXT => Some("String"),
        WRITE | WRITE_TEXT | CLOSE => Some("Nothing"),
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
        READ if exact(arg_types, &[TLS_SOCKET_TYPE, "Integer"]) => Cow::Borrowed("List OF Byte"),
        READ_TEXT if exact(arg_types, &[TLS_SOCKET_TYPE, "Integer"]) => Cow::Borrowed("String"),
        WRITE if exact(arg_types, &[TLS_SOCKET_TYPE, "List OF Byte"]) => Cow::Borrowed("Nothing"),
        WRITE_TEXT if exact(arg_types, &[TLS_SOCKET_TYPE, "String"]) => Cow::Borrowed("Nothing"),
        CLOSE if exact(arg_types, &[TLS_SOCKET_TYPE]) => Cow::Borrowed("Nothing"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        CONNECT => Some("String, Integer, Integer, String"),
        READ | READ_TEXT => Some("TlsSocket, Integer"),
        WRITE => Some("TlsSocket, List OF Byte"),
        WRITE_TEXT => Some("TlsSocket, String"),
        CLOSE => Some("TlsSocket"),
        _ => None,
    }
}

/// Concrete per-argument types for literal coercion. Overloaded/defaulted calls
/// return `None` and rely on explicit argument types.
pub(crate) fn argument_types(name: &str) -> Option<&'static str> {
    match name {
        READ | READ_TEXT => Some("TlsSocket, Integer"),
        WRITE => Some("TlsSocket, List OF Byte"),
        WRITE_TEXT => Some("TlsSocket, String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        CONNECT => Some((2, 4)),
        READ | READ_TEXT | WRITE | WRITE_TEXT => Some((2, 2)),
        CLOSE => Some((1, 1)),
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
    match name {
        // connect(host, port, [timeoutMs=0], [serverName=""])
        CONNECT => &CONNECT_DEFAULTS[provided.saturating_sub(2).min(CONNECT_DEFAULTS.len())..],
        _ => &[],
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `tls.close` consumes the `TlsSocket` it closes.
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!((name, index), (CLOSE, 0))
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
