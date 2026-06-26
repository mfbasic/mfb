use std::borrow::Cow;

pub(crate) const SOCKET_TYPE: &str = "Socket";
pub(crate) const LISTENER_TYPE: &str = "Listener";
pub(crate) const ADDRESS_TYPE: &str = "Address";
pub(crate) const UDP_SOCKET_TYPE: &str = "UdpSocket";
pub(crate) const DATAGRAM_TYPE: &str = "Datagram";
pub(crate) const DATAGRAM_TEXT_TYPE: &str = "DatagramText";

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
        CONNECT_TCP => Some(&[&["host", "address"], &["port", "timeoutMs"], &["timeoutMs"]]),
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
        SET_READ_TIMEOUT | SET_WRITE_TIMEOUT => Some("Socket, Integer"),
        BIND_UDP => Some("String, Integer"),
        RECEIVE_FROM | RECEIVE_TEXT_FROM => Some("UdpSocket, Integer"),
        SEND_TO => Some("UdpSocket, Address, List OF Byte"),
        SEND_TEXT_TO => Some("UdpSocket, Address, String"),
        _ => None,
    }
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
        CLOSE | LOCAL_ADDRESS | REMOTE_ADDRESS => Some((1, 1)),
        _ => None,
    }
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
