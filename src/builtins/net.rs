use std::borrow::Cow;
use std::path::Path;

pub(crate) const SOCKET_TYPE: &str = "Socket";
pub(crate) const LISTENER_TYPE: &str = "Listener";
pub(crate) const ADDRESS_TYPE: &str = "Address";
pub(crate) const UDP_SOCKET_TYPE: &str = "UdpSocket";
pub(crate) const DATAGRAM_TYPE: &str = "Datagram";
pub(crate) const DATAGRAM_TEXT_TYPE: &str = "DatagramText";
/// The `Url` value record lives in the source companion (`net_package.mfb`); it
/// is registered here as a built-in package type (plan-03-http.md §A.1).
pub(crate) const URL_TYPE: &str = "Url";

const LOOKUP: &str = "net.lookup";
const CONNECT_TCP: &str = "net.connectTcp";
const LISTEN_TCP: &str = "net.listenTcp";
const ACCEPT: &str = "net.accept";
const POLL: &str = "net.poll";
const READ: &str = "net.read";
const READ_TEXT: &str = "net.readText";
const WRITE: &str = "net.write";
const WRITE_TEXT: &str = "net.writeText";
const CLOSE: &str = "net.close";
const LOCAL_ADDRESS: &str = "net.localAddress";
const REMOTE_ADDRESS: &str = "net.remoteAddress";
const SET_READ_TIMEOUT: &str = "net.setReadTimeout";
const SET_WRITE_TIMEOUT: &str = "net.setWriteTimeout";
const BIND_UDP: &str = "net.bindUdp";
const RECEIVE_FROM: &str = "net.receiveFrom";
const RECEIVE_TEXT_FROM: &str = "net.receiveTextFrom";
const SEND_TO: &str = "net.sendTo";
const SEND_TEXT_TO: &str = "net.sendTextTo";
// Source-companion calls (`net_package.mfb`): pure URL string work.
const TO_URL: &str = "net.toUrl";
const INTERNAL_TO_URL: &str = "__net_toUrl";
// URL component decoders consumed by the `http` server (plan-05 §F.4.2).
const PERCENT_DECODE: &str = "net.percentDecode";
const INTERNAL_PERCENT_DECODE: &str = "__net_percentDecode";
const PARSE_QUERY: &str = "net.parseQuery";
const INTERNAL_PARSE_QUERY: &str = "__net_parseQuery";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_net_call(name: &str) -> bool {
    matches!(
        name,
        LOOKUP
            | CONNECT_TCP
            | LISTEN_TCP
            | ACCEPT
            | POLL
            | READ
            | READ_TEXT
            | WRITE
            | WRITE_TEXT
            | CLOSE
            | LOCAL_ADDRESS
            | REMOTE_ADDRESS
            | SET_READ_TIMEOUT
            | SET_WRITE_TIMEOUT
            | BIND_UDP
            | RECEIVE_FROM
            | RECEIVE_TEXT_FROM
            | SEND_TO
            | SEND_TEXT_TO
            | TO_URL
            | PERCENT_DECODE
            | PARSE_QUERY
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        SOCKET_TYPE
            | LISTENER_TYPE
            | ADDRESS_TYPE
            | UDP_SOCKET_TYPE
            | DATAGRAM_TYPE
            | DATAGRAM_TEXT_TYPE
            | URL_TYPE
    )
}

pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match name {
        ADDRESS_TYPE => Some(&[("host", "String"), ("port", "Integer")]),
        DATAGRAM_TYPE => Some(&[("from", "Address"), ("bytes", "List OF Byte")]),
        DATAGRAM_TEXT_TYPE => Some(&[("from", "Address"), ("value", "String")]),
        _ => None,
    }
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        SOCKET_TYPE | LISTENER_TYPE | UDP_SOCKET_TYPE => Some(CLOSE),
        _ => None,
    }
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        LOOKUP => Some(&[&["host"], &["port"]]),
        // CONNECT_TCP's overloads do not share a positional layout (`timeoutMs`
        // is param 1 of the Address forms and param 2 of the host/port forms), so
        // it cannot be described by a merged per-position alias table. It has a
        // per-overload table instead; see `call_param_name_overloads`.
        LISTEN_TCP => Some(&[&["host"], &["port"], &["backlog"]]),
        ACCEPT => Some(&[&["listener"], &["timeoutMs"]]),
        POLL => Some(&[&["sock"], &["timeoutMs"]]),
        READ | READ_TEXT => Some(&[&["sock"], &["maxBytes"]]),
        WRITE => Some(&[&["sock"], &["bytes"]]),
        WRITE_TEXT => Some(&[&["sock"], &["value"]]),
        CLOSE => Some(&[&["resource", "sock", "listener"]]),
        LOCAL_ADDRESS => Some(&[&["sock", "listener"]]),
        REMOTE_ADDRESS => Some(&[&["sock"]]),
        SET_READ_TIMEOUT | SET_WRITE_TIMEOUT => Some(&[&["sock"], &["timeoutMs"]]),
        BIND_UDP => Some(&[&["host"], &["port"]]),
        RECEIVE_FROM | RECEIVE_TEXT_FROM => Some(&[&["sock"], &["maxBytes"]]),
        SEND_TO => Some(&[&["sock"], &["address"], &["bytes"]]),
        SEND_TEXT_TO => Some(&[&["sock"], &["address"], &["value"]]),
        TO_URL => Some(&[&["href", "value", "url"]]),
        PERCENT_DECODE => Some(&[&["s", "text", "value"]]),
        PARSE_QUERY => Some(&[&["s", "query", "value"]]),
        _ => None,
    }
}

/// Per-overload parameter names for a builtin whose overloads have structurally
/// different positional layouts, so a named argument binds to a different index
/// depending on which overload it selects. Each entry is one overload's parameter
/// names, in order.
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        CONNECT_TCP => Some(&[
            &["host", "port"],
            &["host", "port", "timeoutMs"],
            &["address"],
            &["address", "timeoutMs"],
        ]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        LOOKUP => Some("List OF Address"),
        CONNECT_TCP | ACCEPT => Some(SOCKET_TYPE),
        LISTEN_TCP => Some(LISTENER_TYPE),
        // `poll` is overloaded: `Boolean` for a single socket and `List OF
        // Boolean` for a list. `resolve_call` returns the precise type; this
        // nominal value only flags the call as a recognized builtin.
        POLL => Some("Boolean"),
        READ => Some("List OF Byte"),
        READ_TEXT => Some("String"),
        WRITE | WRITE_TEXT | CLOSE | SET_READ_TIMEOUT | SET_WRITE_TIMEOUT | SEND_TO
        | SEND_TEXT_TO => Some("Nothing"),
        LOCAL_ADDRESS | REMOTE_ADDRESS => Some(ADDRESS_TYPE),
        BIND_UDP => Some(UDP_SOCKET_TYPE),
        RECEIVE_FROM => Some(DATAGRAM_TYPE),
        RECEIVE_TEXT_FROM => Some(DATAGRAM_TEXT_TYPE),
        TO_URL => Some(URL_TYPE),
        PERCENT_DECODE => Some("String"),
        PARSE_QUERY => Some("Map OF String TO String"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        LOOKUP if exact(arg_types, &["String"]) || exact(arg_types, &["String", "Integer"]) => {
            Cow::Borrowed("List OF Address")
        }
        CONNECT_TCP
            if exact(arg_types, &["String", "Integer"])
                || exact(arg_types, &["String", "Integer", "Integer"])
                || exact(arg_types, &[ADDRESS_TYPE])
                || exact(arg_types, &[ADDRESS_TYPE, "Integer"]) =>
        {
            Cow::Borrowed(SOCKET_TYPE)
        }
        LISTEN_TCP
            if exact(arg_types, &["String", "Integer"])
                || exact(arg_types, &["String", "Integer", "Integer"]) =>
        {
            Cow::Borrowed(LISTENER_TYPE)
        }
        ACCEPT
            if exact(arg_types, &[LISTENER_TYPE])
                || exact(arg_types, &[LISTENER_TYPE, "Integer"]) =>
        {
            Cow::Borrowed(SOCKET_TYPE)
        }
        // The `poll(List OF Socket)` overload in the specification is omitted: the
        // ownership model forbids resource handles as collection elements, so a
        // `List OF Socket` value cannot be constructed and the overload is
        // unreachable. Single-socket readiness polling is provided here.
        POLL if exact(arg_types, &[SOCKET_TYPE]) || exact(arg_types, &[SOCKET_TYPE, "Integer"]) => {
            Cow::Borrowed("Boolean")
        }
        READ if exact(arg_types, &[SOCKET_TYPE, "Integer"]) => Cow::Borrowed("List OF Byte"),
        READ_TEXT if exact(arg_types, &[SOCKET_TYPE, "Integer"]) => Cow::Borrowed("String"),
        WRITE if exact(arg_types, &[SOCKET_TYPE, "List OF Byte"]) => Cow::Borrowed("Nothing"),
        WRITE_TEXT if exact(arg_types, &[SOCKET_TYPE, "String"]) => Cow::Borrowed("Nothing"),
        CLOSE if exact(arg_types, &[SOCKET_TYPE]) || exact(arg_types, &[LISTENER_TYPE]) => {
            Cow::Borrowed("Nothing")
        }
        LOCAL_ADDRESS if exact(arg_types, &[SOCKET_TYPE]) || exact(arg_types, &[LISTENER_TYPE]) => {
            Cow::Borrowed(ADDRESS_TYPE)
        }
        REMOTE_ADDRESS if exact(arg_types, &[SOCKET_TYPE]) => Cow::Borrowed(ADDRESS_TYPE),
        SET_READ_TIMEOUT | SET_WRITE_TIMEOUT
            if exact(arg_types, &[SOCKET_TYPE, "Integer"])
                || exact(arg_types, &[UDP_SOCKET_TYPE, "Integer"]) =>
        {
            Cow::Borrowed("Nothing")
        }
        // UDP datagram sockets.
        BIND_UDP if exact(arg_types, &["String", "Integer"]) => Cow::Borrowed(UDP_SOCKET_TYPE),
        RECEIVE_FROM if exact(arg_types, &[UDP_SOCKET_TYPE, "Integer"]) => {
            Cow::Borrowed(DATAGRAM_TYPE)
        }
        RECEIVE_TEXT_FROM if exact(arg_types, &[UDP_SOCKET_TYPE, "Integer"]) => {
            Cow::Borrowed(DATAGRAM_TEXT_TYPE)
        }
        SEND_TO if exact(arg_types, &[UDP_SOCKET_TYPE, ADDRESS_TYPE, "List OF Byte"]) => {
            Cow::Borrowed("Nothing")
        }
        SEND_TEXT_TO if exact(arg_types, &[UDP_SOCKET_TYPE, ADDRESS_TYPE, "String"]) => {
            Cow::Borrowed("Nothing")
        }
        // `close`/`localAddress` are also overloaded on `UdpSocket`.
        CLOSE if exact(arg_types, &[UDP_SOCKET_TYPE]) => Cow::Borrowed("Nothing"),
        LOCAL_ADDRESS if exact(arg_types, &[UDP_SOCKET_TYPE]) => Cow::Borrowed(ADDRESS_TYPE),
        TO_URL if exact(arg_types, &["String"]) => Cow::Borrowed(URL_TYPE),
        PERCENT_DECODE if exact(arg_types, &["String"]) => Cow::Borrowed("String"),
        PARSE_QUERY if exact(arg_types, &["String"]) => Cow::Borrowed("Map OF String TO String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        LOOKUP => Some("String, Integer"),
        CONNECT_TCP => Some("String, Integer, Integer or Address, Integer"),
        LISTEN_TCP => Some("String, Integer, Integer"),
        ACCEPT => Some("Listener, Integer"),
        POLL => Some("Socket, Integer"),
        READ => Some("Socket, Integer"),
        READ_TEXT => Some("Socket, Integer"),
        WRITE => Some("Socket, List OF Byte"),
        WRITE_TEXT => Some("Socket, String"),
        CLOSE => Some("Socket or Listener or UdpSocket"),
        LOCAL_ADDRESS => Some("Socket or Listener or UdpSocket"),
        REMOTE_ADDRESS => Some("Socket"),
        SET_READ_TIMEOUT | SET_WRITE_TIMEOUT => Some("Socket or UdpSocket, Integer"),
        BIND_UDP => Some("String, Integer"),
        RECEIVE_FROM | RECEIVE_TEXT_FROM => Some("UdpSocket, Integer"),
        SEND_TO => Some("UdpSocket, Address, List OF Byte"),
        SEND_TEXT_TO => Some("UdpSocket, Address, String"),
        TO_URL => Some("String"),
        PERCENT_DECODE => Some("String"),
        PARSE_QUERY => Some("String"),
        _ => None,
    }
}

/// Concrete per-argument types for literal coercion (e.g. typing a `[1, 2]`
/// list literal as `List OF Byte`). Only the non-overloaded calls return a
/// machine-splittable signature; overloaded calls (`connectTcp`, `poll`,
/// `close`, `localAddress`) return `None` and rely on explicit argument types.
pub(crate) fn argument_types(name: &str) -> Option<&'static str> {
    match name {
        LOOKUP => Some("String, Integer"),
        LISTEN_TCP => Some("String, Integer, Integer"),
        ACCEPT => Some("Listener, Integer"),
        READ | READ_TEXT => Some("Socket, Integer"),
        WRITE => Some("Socket, List OF Byte"),
        WRITE_TEXT => Some("Socket, String"),
        REMOTE_ADDRESS => Some("Socket"),
        // Overloaded on `Socket|UdpSocket` — per the doc above, overloaded calls
        // must return `None` and rely on explicit argument types (bug-173 D).
        BIND_UDP => Some("String, Integer"),
        RECEIVE_FROM | RECEIVE_TEXT_FROM => Some("UdpSocket, Integer"),
        SEND_TO => Some("UdpSocket, Address, List OF Byte"),
        SEND_TEXT_TO => Some("UdpSocket, Address, String"),
        TO_URL => Some("String"),
        PERCENT_DECODE => Some("String"),
        PARSE_QUERY => Some("String"),
        _ => None,
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `net.close` consumes the socket/listener handle it closes; every other
/// call only uses its handle, which stays open.
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!((name, index), (CLOSE, 0))
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        LOOKUP => Some((1, 2)),
        CONNECT_TCP => Some((1, 3)),
        LISTEN_TCP => Some((2, 3)),
        ACCEPT => Some((1, 2)),
        POLL => Some((1, 2)),
        READ | READ_TEXT | WRITE | WRITE_TEXT | SET_READ_TIMEOUT | SET_WRITE_TIMEOUT | BIND_UDP
        | RECEIVE_FROM | RECEIVE_TEXT_FROM => Some((2, 2)),
        SEND_TO | SEND_TEXT_TO => Some((3, 3)),
        CLOSE | LOCAL_ADDRESS | REMOTE_ADDRESS | TO_URL | PERCENT_DECODE | PARSE_QUERY => {
            Some((1, 1))
        }
        _ => None,
    }
}

/// The internal source-companion target for a source-backed `net` call
/// (`net_package.mfb`). Native calls (sockets/DNS/UDP) return `None` and stay
/// `net.*` runtime-helper calls.
pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    match name {
        TO_URL => Some(INTERNAL_TO_URL),
        PERCENT_DECODE => Some(INTERNAL_PERCENT_DECODE),
        PARSE_QUERY => Some(INTERNAL_PARSE_QUERY),
        _ => None,
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-net>"),
        "builtins/net.mfb",
        include_str!("net_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "net")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }

    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
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

    fn ret(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn is_net_call_flags() {
        for f in [
            LOOKUP,
            CONNECT_TCP,
            LISTEN_TCP,
            ACCEPT,
            POLL,
            READ,
            READ_TEXT,
            WRITE,
            WRITE_TEXT,
            CLOSE,
            LOCAL_ADDRESS,
            REMOTE_ADDRESS,
            SET_READ_TIMEOUT,
            SET_WRITE_TIMEOUT,
            BIND_UDP,
            RECEIVE_FROM,
            RECEIVE_TEXT_FROM,
            SEND_TO,
            SEND_TEXT_TO,
            TO_URL,
        ] {
            assert!(is_net_call(f), "{f}");
        }
        assert!(!is_net_call(INTERNAL_TO_URL));
        assert!(!is_net_call("net.bogus"));
    }

    #[test]
    fn builtin_types_and_fields() {
        for t in [
            SOCKET_TYPE,
            LISTENER_TYPE,
            ADDRESS_TYPE,
            UDP_SOCKET_TYPE,
            DATAGRAM_TYPE,
            DATAGRAM_TEXT_TYPE,
            URL_TYPE,
        ] {
            assert!(is_builtin_type(t), "{t}");
        }
        assert!(!is_builtin_type("Socketx"));

        assert_eq!(
            builtin_type_fields(ADDRESS_TYPE),
            Some(&[("host", "String"), ("port", "Integer")][..])
        );
        assert_eq!(
            builtin_type_fields(DATAGRAM_TYPE),
            Some(&[("from", "Address"), ("bytes", "List OF Byte")][..])
        );
        assert_eq!(
            builtin_type_fields(DATAGRAM_TEXT_TYPE),
            Some(&[("from", "Address"), ("value", "String")][..])
        );
        assert_eq!(builtin_type_fields(SOCKET_TYPE), None);
        assert_eq!(builtin_type_fields(URL_TYPE), None);
    }

    #[test]
    fn resource_close_functions() {
        assert_eq!(resource_close_function(SOCKET_TYPE), Some(CLOSE));
        assert_eq!(resource_close_function(LISTENER_TYPE), Some(CLOSE));
        assert_eq!(resource_close_function(UDP_SOCKET_TYPE), Some(CLOSE));
        assert_eq!(resource_close_function(ADDRESS_TYPE), None);
        assert_eq!(resource_close_function(URL_TYPE), None);
    }

    #[test]
    fn call_param_names_present_and_absent() {
        assert!(call_param_names(LOOKUP).is_some());
        // CONNECT_TCP carries a per-overload table instead of a merged one.
        assert!(call_param_names(CONNECT_TCP).is_none());
        assert!(call_param_name_overloads(CONNECT_TCP).is_some());
        assert!(call_param_name_overloads(LOOKUP).is_none());
        assert!(call_param_names(LISTEN_TCP).is_some());
        assert!(call_param_names(ACCEPT).is_some());
        assert!(call_param_names(POLL).is_some());
        assert!(call_param_names(READ).is_some());
        assert!(call_param_names(WRITE).is_some());
        assert!(call_param_names(WRITE_TEXT).is_some());
        assert!(call_param_names(CLOSE).is_some());
        assert!(call_param_names(LOCAL_ADDRESS).is_some());
        assert!(call_param_names(REMOTE_ADDRESS).is_some());
        assert!(call_param_names(SET_READ_TIMEOUT).is_some());
        assert!(call_param_names(BIND_UDP).is_some());
        assert!(call_param_names(RECEIVE_FROM).is_some());
        assert!(call_param_names(SEND_TO).is_some());
        assert!(call_param_names(SEND_TEXT_TO).is_some());
        assert!(call_param_names(TO_URL).is_some());
        assert_eq!(call_param_names("net.bogus"), None);
    }

    #[test]
    fn call_return_type_names() {
        assert_eq!(call_return_type_name(LOOKUP), Some("List OF Address"));
        assert_eq!(call_return_type_name(CONNECT_TCP), Some(SOCKET_TYPE));
        assert_eq!(call_return_type_name(ACCEPT), Some(SOCKET_TYPE));
        assert_eq!(call_return_type_name(LISTEN_TCP), Some(LISTENER_TYPE));
        assert_eq!(call_return_type_name(POLL), Some("Boolean"));
        assert_eq!(call_return_type_name(READ), Some("List OF Byte"));
        assert_eq!(call_return_type_name(READ_TEXT), Some("String"));
        assert_eq!(call_return_type_name(WRITE), Some("Nothing"));
        assert_eq!(call_return_type_name(SEND_TO), Some("Nothing"));
        assert_eq!(call_return_type_name(LOCAL_ADDRESS), Some(ADDRESS_TYPE));
        assert_eq!(call_return_type_name(REMOTE_ADDRESS), Some(ADDRESS_TYPE));
        assert_eq!(call_return_type_name(BIND_UDP), Some(UDP_SOCKET_TYPE));
        assert_eq!(call_return_type_name(RECEIVE_FROM), Some(DATAGRAM_TYPE));
        assert_eq!(
            call_return_type_name(RECEIVE_TEXT_FROM),
            Some(DATAGRAM_TEXT_TYPE)
        );
        assert_eq!(call_return_type_name(TO_URL), Some(URL_TYPE));
        assert_eq!(call_return_type_name("net.bogus"), None);
    }

    #[test]
    fn resolve_lookup() {
        assert_eq!(
            ret(LOOKUP, &["String"]),
            Some("List OF Address".to_string())
        );
        assert_eq!(
            ret(LOOKUP, &["String", "Integer"]),
            Some("List OF Address".to_string())
        );
        assert_eq!(ret(LOOKUP, &["Integer"]), None);
        assert_eq!(ret(LOOKUP, &[]), None);
    }

    #[test]
    fn resolve_connect_tcp_overloads() {
        assert_eq!(
            ret(CONNECT_TCP, &["String", "Integer"]),
            Some(SOCKET_TYPE.to_string())
        );
        assert_eq!(
            ret(CONNECT_TCP, &["String", "Integer", "Integer"]),
            Some(SOCKET_TYPE.to_string())
        );
        assert_eq!(
            ret(CONNECT_TCP, &[ADDRESS_TYPE]),
            Some(SOCKET_TYPE.to_string())
        );
        assert_eq!(
            ret(CONNECT_TCP, &[ADDRESS_TYPE, "Integer"]),
            Some(SOCKET_TYPE.to_string())
        );
        assert_eq!(ret(CONNECT_TCP, &["Integer"]), None);
    }

    #[test]
    fn resolve_listen_and_accept() {
        assert_eq!(
            ret(LISTEN_TCP, &["String", "Integer"]),
            Some(LISTENER_TYPE.to_string())
        );
        assert_eq!(
            ret(LISTEN_TCP, &["String", "Integer", "Integer"]),
            Some(LISTENER_TYPE.to_string())
        );
        assert_eq!(ret(LISTEN_TCP, &["String"]), None);
        assert_eq!(ret(ACCEPT, &[LISTENER_TYPE]), Some(SOCKET_TYPE.to_string()));
        assert_eq!(
            ret(ACCEPT, &[LISTENER_TYPE, "Integer"]),
            Some(SOCKET_TYPE.to_string())
        );
        assert_eq!(ret(ACCEPT, &[SOCKET_TYPE]), None);
    }

    #[test]
    fn resolve_poll_and_io() {
        assert_eq!(ret(POLL, &[SOCKET_TYPE]), Some("Boolean".to_string()));
        assert_eq!(
            ret(POLL, &[SOCKET_TYPE, "Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(ret(POLL, &[LISTENER_TYPE]), None);
        assert_eq!(
            ret(READ, &[SOCKET_TYPE, "Integer"]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            ret(READ_TEXT, &[SOCKET_TYPE, "Integer"]),
            Some("String".to_string())
        );
        assert_eq!(
            ret(WRITE, &[SOCKET_TYPE, "List OF Byte"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            ret(WRITE_TEXT, &[SOCKET_TYPE, "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(READ, &[SOCKET_TYPE]), None);
        assert_eq!(ret(WRITE, &[SOCKET_TYPE, "String"]), None);
    }

    #[test]
    fn resolve_close_and_addresses() {
        assert_eq!(ret(CLOSE, &[SOCKET_TYPE]), Some("Nothing".to_string()));
        assert_eq!(ret(CLOSE, &[LISTENER_TYPE]), Some("Nothing".to_string()));
        assert_eq!(ret(CLOSE, &[UDP_SOCKET_TYPE]), Some("Nothing".to_string()));
        assert_eq!(ret(CLOSE, &[ADDRESS_TYPE]), None);
        assert_eq!(
            ret(LOCAL_ADDRESS, &[SOCKET_TYPE]),
            Some(ADDRESS_TYPE.to_string())
        );
        assert_eq!(
            ret(LOCAL_ADDRESS, &[LISTENER_TYPE]),
            Some(ADDRESS_TYPE.to_string())
        );
        assert_eq!(
            ret(LOCAL_ADDRESS, &[UDP_SOCKET_TYPE]),
            Some(ADDRESS_TYPE.to_string())
        );
        assert_eq!(
            ret(REMOTE_ADDRESS, &[SOCKET_TYPE]),
            Some(ADDRESS_TYPE.to_string())
        );
        assert_eq!(ret(REMOTE_ADDRESS, &[LISTENER_TYPE]), None);
    }

    #[test]
    fn resolve_timeouts() {
        assert_eq!(
            ret(SET_READ_TIMEOUT, &[SOCKET_TYPE, "Integer"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            ret(SET_WRITE_TIMEOUT, &[UDP_SOCKET_TYPE, "Integer"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(SET_READ_TIMEOUT, &[SOCKET_TYPE]), None);
    }

    #[test]
    fn resolve_udp() {
        assert_eq!(
            ret(BIND_UDP, &["String", "Integer"]),
            Some(UDP_SOCKET_TYPE.to_string())
        );
        assert_eq!(
            ret(RECEIVE_FROM, &[UDP_SOCKET_TYPE, "Integer"]),
            Some(DATAGRAM_TYPE.to_string())
        );
        assert_eq!(
            ret(RECEIVE_TEXT_FROM, &[UDP_SOCKET_TYPE, "Integer"]),
            Some(DATAGRAM_TEXT_TYPE.to_string())
        );
        assert_eq!(
            ret(SEND_TO, &[UDP_SOCKET_TYPE, ADDRESS_TYPE, "List OF Byte"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            ret(SEND_TEXT_TO, &[UDP_SOCKET_TYPE, ADDRESS_TYPE, "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(BIND_UDP, &["String"]), None);
        assert_eq!(
            ret(SEND_TO, &[UDP_SOCKET_TYPE, ADDRESS_TYPE, "String"]),
            None
        );
    }

    #[test]
    fn resolve_to_url_and_unknown() {
        assert_eq!(ret(TO_URL, &["String"]), Some(URL_TYPE.to_string()));
        assert_eq!(ret(TO_URL, &["Integer"]), None);
        assert_eq!(ret("net.bogus", &["String"]), None);
    }

    #[test]
    fn expected_arguments_present() {
        assert_eq!(expected_arguments(LOOKUP), Some("String, Integer"));
        assert!(expected_arguments(CONNECT_TCP).unwrap().contains("Address"));
        assert!(expected_arguments(CLOSE).unwrap().contains("UdpSocket"));
        assert_eq!(expected_arguments(REMOTE_ADDRESS), Some("Socket"));
        assert!(expected_arguments(SET_READ_TIMEOUT).is_some());
        assert!(expected_arguments(SEND_TO).is_some());
        assert!(expected_arguments(SEND_TEXT_TO).is_some());
        assert_eq!(expected_arguments(TO_URL), Some("String"));
        assert_eq!(expected_arguments("net.bogus"), None);
    }

    #[test]
    fn argument_types_present_and_none() {
        assert_eq!(argument_types(LOOKUP), Some("String, Integer"));
        assert_eq!(argument_types(LISTEN_TCP), Some("String, Integer, Integer"));
        assert_eq!(argument_types(ACCEPT), Some("Listener, Integer"));
        assert_eq!(argument_types(READ), Some("Socket, Integer"));
        assert_eq!(argument_types(WRITE), Some("Socket, List OF Byte"));
        assert_eq!(argument_types(REMOTE_ADDRESS), Some("Socket"));
        assert!(argument_types(BIND_UDP).is_some());
        assert!(argument_types(SEND_TO).is_some());
        assert_eq!(argument_types(TO_URL), Some("String"));
        // overloaded calls return None (bug-173 D: the timeout setters are
        // overloaded on `Socket|UdpSocket`)
        assert_eq!(argument_types(SET_READ_TIMEOUT), None);
        assert_eq!(argument_types(SET_WRITE_TIMEOUT), None);
        assert_eq!(argument_types(CONNECT_TCP), None);
        assert_eq!(argument_types(POLL), None);
        assert_eq!(argument_types(CLOSE), None);
        assert_eq!(argument_types(LOCAL_ADDRESS), None);
        assert_eq!(argument_types("net.bogus"), None);
    }

    #[test]
    fn arity_spans() {
        assert_eq!(arity(LOOKUP), Some((1, 2)));
        assert_eq!(arity(CONNECT_TCP), Some((1, 3)));
        assert_eq!(arity(LISTEN_TCP), Some((2, 3)));
        assert_eq!(arity(ACCEPT), Some((1, 2)));
        assert_eq!(arity(POLL), Some((1, 2)));
        assert_eq!(arity(READ), Some((2, 2)));
        assert_eq!(arity(SEND_TO), Some((3, 3)));
        assert_eq!(arity(CLOSE), Some((1, 1)));
        assert_eq!(arity(TO_URL), Some((1, 1)));
        assert_eq!(arity("net.bogus"), None);
    }

    #[test]
    fn implementation_name_to_url_only() {
        assert_eq!(implementation_name(TO_URL), Some(INTERNAL_TO_URL));
        assert_eq!(implementation_name(LOOKUP), None);
        assert_eq!(implementation_name("net.bogus"), None);
    }

    #[test]
    fn exact_helper() {
        assert!(exact(
            &strings(&["String", "Integer"]),
            &["String", "Integer"]
        ));
        assert!(!exact(&strings(&["String"]), &["String", "Integer"]));
        assert!(!exact(&strings(&["Integer"]), &["String"]));
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT net\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }
}
