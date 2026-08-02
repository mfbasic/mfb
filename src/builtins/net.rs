use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    BuiltinType, DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType, TypeKind,
};

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

// plan-72-R: `NET` is the descriptor authority for this package. net is data-only
// (no resolver): every call's return type is fixed per name (the overloading is on
// ARGUMENT types, not the return), so `call_return_type_name` and `arity` derive
// from the descriptor. `connectTcp`'s four structurally-different overloads are
// modelled as four `BuiltinOverload`s, so `DefaultResolver::param_name_overloads`
// reproduces the legacy per-overload name table (and `param_names` correctly
// yields `None`). `implementation_name` is a fixed per-name rewrite
// (`Implementation::Rewrite(__net_*)`) for the three source-companion calls and
// `Same` elsewhere. The seven builtin types include three records (Address,
// Datagram, DatagramText). The `resolve_call` (type-set argument acceptance:
// `close(Socket|Listener|UdpSocket)`, `connectTcp(String,Integer | Address)`),
// `expected_arguments` (`"or"`-phrased), and `argument_types` (joined strings,
// overloaded → `None`) stay hand-authored — the descriptor's exact per-position
// match and per-type rendering cannot reproduce them.
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
    implementation: Implementation,
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation,
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

// An optional trailing parameter: widens the arity range but is NOT
// default-padded (net has no `default_argument_padding` helper), so `Optional`,
// not `Fill`.
const fn opt(name: &'static str, ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Optional,
    }
}

const P_LOOKUP: &[Parameter] = &[req("host", &[], "String"), opt("port", "Integer")];
const P_CT_HP: &[Parameter] = &[req("host", &[], "String"), req("port", &[], "Integer")];
const P_CT_HPT: &[Parameter] = &[
    req("host", &[], "String"),
    req("port", &[], "Integer"),
    req("timeoutMs", &[], "Integer"),
];
const P_CT_A: &[Parameter] = &[req("address", &[], ADDRESS_TYPE)];
const P_CT_AT: &[Parameter] = &[req("address", &[], ADDRESS_TYPE), req("timeoutMs", &[], "Integer")];
const P_LISTEN: &[Parameter] = &[
    req("host", &[], "String"),
    req("port", &[], "Integer"),
    opt("backlog", "Integer"),
];
const P_ACCEPT: &[Parameter] = &[req("listener", &[], LISTENER_TYPE), opt("timeoutMs", "Integer")];
const P_POLL: &[Parameter] = &[req("sock", &[], SOCKET_TYPE), opt("timeoutMs", "Integer")];
const P_READ: &[Parameter] = &[req("sock", &[], SOCKET_TYPE), req("maxBytes", &[], "Integer")];
const P_WRITE: &[Parameter] = &[req("sock", &[], SOCKET_TYPE), req("bytes", &[], "List OF Byte")];
const P_WRITE_TEXT: &[Parameter] = &[req("sock", &[], SOCKET_TYPE), req("value", &[], "String")];
const P_CLOSE: &[Parameter] = &[req("resource", &["sock", "listener"], SOCKET_TYPE)];
const P_LOCAL_ADDR: &[Parameter] = &[req("sock", &["listener"], SOCKET_TYPE)];
const P_REMOTE_ADDR: &[Parameter] = &[req("sock", &[], SOCKET_TYPE)];
const P_TIMEOUT_SET: &[Parameter] =
    &[req("sock", &[], SOCKET_TYPE), req("timeoutMs", &[], "Integer")];
const P_BIND_UDP: &[Parameter] = &[req("host", &[], "String"), req("port", &[], "Integer")];
const P_RECV: &[Parameter] = &[req("sock", &[], UDP_SOCKET_TYPE), req("maxBytes", &[], "Integer")];
const P_SEND: &[Parameter] = &[
    req("sock", &[], UDP_SOCKET_TYPE),
    req("address", &[], ADDRESS_TYPE),
    req("bytes", &[], "List OF Byte"),
];
const P_SEND_TEXT: &[Parameter] = &[
    req("sock", &[], UDP_SOCKET_TYPE),
    req("address", &[], ADDRESS_TYPE),
    req("value", &[], "String"),
];
const P_TO_URL: &[Parameter] = &[req("href", &["value", "url"], "String")];
const P_PERCENT: &[Parameter] = &[req("s", &["text", "value"], "String")];
const P_PARSE_QUERY: &[Parameter] = &[req("s", &["query", "value"], "String")];

// `connectTcp`'s four overloads have structurally different positional layouts;
// modelling them as four overloads makes `DefaultResolver::param_name_overloads`
// reproduce the legacy per-overload name table and `param_names` yield `None`.
const OV_CONNECT: &[BuiltinOverload] = &[
    ov(P_CT_HP, SOCKET_TYPE),
    ov(P_CT_HPT, SOCKET_TYPE),
    ov(P_CT_A, SOCKET_TYPE),
    ov(P_CT_AT, SOCKET_TYPE),
];

const NET_FUNCTIONS: &[BuiltinFunction] = &[
    nf(LOOKUP, "lookup", &[ov(P_LOOKUP, "List OF Address")], Implementation::Same),
    nf(CONNECT_TCP, "connectTcp", OV_CONNECT, Implementation::Same),
    nf(LISTEN_TCP, "listenTcp", &[ov(P_LISTEN, LISTENER_TYPE)], Implementation::Same),
    nf(ACCEPT, "accept", &[ov(P_ACCEPT, SOCKET_TYPE)], Implementation::Same),
    nf(POLL, "poll", &[ov(P_POLL, "Boolean")], Implementation::Same),
    nf(READ, "read", &[ov(P_READ, "List OF Byte")], Implementation::Same),
    nf(READ_TEXT, "readText", &[ov(P_READ, "String")], Implementation::Same),
    nf(WRITE, "write", &[ov(P_WRITE, "Nothing")], Implementation::Same),
    nf(WRITE_TEXT, "writeText", &[ov(P_WRITE_TEXT, "Nothing")], Implementation::Same),
    nf(CLOSE, "close", &[ov(P_CLOSE, "Nothing")], Implementation::Same),
    nf(LOCAL_ADDRESS, "localAddress", &[ov(P_LOCAL_ADDR, ADDRESS_TYPE)], Implementation::Same),
    nf(REMOTE_ADDRESS, "remoteAddress", &[ov(P_REMOTE_ADDR, ADDRESS_TYPE)], Implementation::Same),
    nf(SET_READ_TIMEOUT, "setReadTimeout", &[ov(P_TIMEOUT_SET, "Nothing")], Implementation::Same),
    nf(SET_WRITE_TIMEOUT, "setWriteTimeout", &[ov(P_TIMEOUT_SET, "Nothing")], Implementation::Same),
    nf(BIND_UDP, "bindUdp", &[ov(P_BIND_UDP, UDP_SOCKET_TYPE)], Implementation::Same),
    nf(RECEIVE_FROM, "receiveFrom", &[ov(P_RECV, DATAGRAM_TYPE)], Implementation::Same),
    nf(RECEIVE_TEXT_FROM, "receiveTextFrom", &[ov(P_RECV, DATAGRAM_TEXT_TYPE)], Implementation::Same),
    nf(SEND_TO, "sendTo", &[ov(P_SEND, "Nothing")], Implementation::Same),
    nf(SEND_TEXT_TO, "sendTextTo", &[ov(P_SEND_TEXT, "Nothing")], Implementation::Same),
    nf(TO_URL, "toUrl", &[ov(P_TO_URL, URL_TYPE)], Implementation::Rewrite(INTERNAL_TO_URL)),
    nf(
        PERCENT_DECODE,
        "percentDecode",
        &[ov(P_PERCENT, "String")],
        Implementation::Rewrite(INTERNAL_PERCENT_DECODE),
    ),
    nf(
        PARSE_QUERY,
        "parseQuery",
        &[ov(P_PARSE_QUERY, "Map OF String TO String")],
        Implementation::Rewrite(INTERNAL_PARSE_QUERY),
    ),
];

const ADDRESS_FIELDS: &[(&str, &str)] = &[("host", "String"), ("port", "Integer")];
const DATAGRAM_FIELDS: &[(&str, &str)] = &[("from", "Address"), ("bytes", "List OF Byte")];
const DATAGRAM_TEXT_FIELDS: &[(&str, &str)] = &[("from", "Address"), ("value", "String")];

const NET_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: SOCKET_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: LISTENER_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: ADDRESS_TYPE,
        kind: TypeKind::Record,
        fields: ADDRESS_FIELDS,
    },
    BuiltinType {
        name: UDP_SOCKET_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: DATAGRAM_TYPE,
        kind: TypeKind::Record,
        fields: DATAGRAM_FIELDS,
    },
    BuiltinType {
        name: DATAGRAM_TEXT_TYPE,
        kind: TypeKind::Record,
        fields: DATAGRAM_TEXT_FIELDS,
    },
    BuiltinType {
        name: URL_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
];

/// Return-type resolution for the net calls, delegating to the hand-authored
/// `resolve_call` (which validates `close`'s `Socket`/`Listener`/`UdpSocket`
/// argument union and `connectTcp`'s `String,Integer`/`Address` overloads that the
/// descriptor's per-position match cannot). Exposed through the descriptor so
/// plan-72-BB can drive `net::` return types from the registry. Per-name
/// implementation rewrites stay data-derivable (`DefaultResolver::implementation_name`),
/// so this resolver does not override `implementation_name`.
struct NetResolver;
impl BuiltinResolver for NetResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        resolve_call(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static NET_RESOLVER: NetResolver = NetResolver;

pub(crate) static NET: BuiltinModule = BuiltinModule {
    name: "net",
    functions: NET_FUNCTIONS,
    types: NET_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&NET_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_net_call(name: &str) -> bool {
    DefaultResolver::contains(&NET, name)
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    NET.types.iter().any(|ty| ty.name == name)
}

// `builtin_type_fields` returns a `&'static` borrowed shape and reports an
// opaque/record's fields; it stays a static literal PINNED equal to `NET`'s type
// records by `parity_matches_descriptor` (`Url` is a record whose fields live in
// the source companion, so like the legacy helper it reports `None` here).
pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match name {
        ADDRESS_TYPE => Some(ADDRESS_FIELDS),
        DATAGRAM_TYPE => Some(DATAGRAM_FIELDS),
        DATAGRAM_TEXT_TYPE => Some(DATAGRAM_TEXT_FIELDS),
        _ => None,
    }
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        SOCKET_TYPE | LISTENER_TYPE | UDP_SOCKET_TYPE => Some(CLOSE),
        _ => None,
    }
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver` (which yields `Vec`) cannot produce, so it stays a static
// literal PINNED equal to `NET` by `parity_matches_descriptor`. `connectTcp`'s
// overloads do not share a positional layout, so it returns `None` here and its
// names live in `call_param_name_overloads`.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        LOOKUP => Some(&[&["host"], &["port"]]),
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
/// different positional layouts. `connectTcp`'s `timeoutMs` is param 1 of the
/// `Address` forms and param 2 of the host/port forms, so a named argument binds
/// to a different index depending on which overload it selects. Returns a
/// `&'static` borrowed shape PINNED equal to `NET`'s four overloads by
/// `parity_matches_descriptor`.
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
    DefaultResolver::return_type_name(&NET, name)
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

// `expected_arguments` uses bespoke `"or"`-phrased strings the descriptor's
// per-position type rendering cannot reproduce, so it stays hand-authored (not
// descriptor-derived); the parity harness opts out of this row. BB removes it.
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
/// A joined-string shape the descriptor's per-position types cannot render, so it
/// stays hand-authored (the parity harness opts out of this row).
pub(crate) fn argument_types(name: &str) -> Option<&'static str> {
    match name {
        LOOKUP => Some("String, Integer"),
        LISTEN_TCP => Some("String, Integer, Integer"),
        ACCEPT => Some("Listener, Integer"),
        READ | READ_TEXT => Some("Socket, Integer"),
        WRITE => Some("Socket, List OF Byte"),
        WRITE_TEXT => Some("Socket, String"),
        REMOTE_ADDRESS => Some("Socket"),
        BIND_UDP => Some("String, Integer"),
        RECEIVE_FROM | RECEIVE_TEXT_FROM => Some("UdpSocket, Integer"),
        SEND_TO => Some("UdpSocket, Address, List OF Byte"),
        SEND_TEXT_TO => Some("UdpSocket, Address, String"),
        TO_URL => Some("String"),
        PERCENT_DECODE => Some("String"),
        PARSE_QUERY => Some("String"),
        // The overloaded calls listed in the doc above (plus `setReadTimeout` /
        // `setWriteTimeout`, overloaded on `Socket|UdpSocket`) fall through here:
        // they must return `None` and rely on explicit argument types (bug-173 D).
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
    DefaultResolver::arity(&NET, name)
}

/// The internal source-companion target for a source-backed `net` call
/// (`net_package.mfb`). Native calls (sockets/DNS/UDP) return `None` and stay
/// `net.*` runtime-helper calls. Derived from the descriptor's per-name
/// `Implementation::Rewrite`.
pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    DefaultResolver::implementation_name(&NET, name)
}

super::package_source_glue!(
    "net",
    "<builtin-net>",
    "builtins/net.mfb",
    include_str!("net_package.mfb")
);

use super::exact;

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
        assert_eq!(implementation_name(PERCENT_DECODE), Some(INTERNAL_PERCENT_DECODE));
        assert_eq!(implementation_name(PARSE_QUERY), Some(INTERNAL_PARSE_QUERY));
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

    // plan-72-R migration gate: prove `NET` reproduces every legacy helper answer
    // for every `net.*` name (and an unknown name) — membership, arity, param
    // names, per-overload param names (connectTcp's four overloads), nominal
    // return type, per-name implementation rewrite, and builtin-type record
    // fields — pinning the borrowed statics equal to `NET`. `resolve_call` (type-set
    // argument acceptance), `expected_arguments` (`"or"`-phrased), and
    // `argument_types` (joined strings) are not descriptor-derivable and are
    // checked by the dedicated tests above. Keep until plan-72-BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let calls: Vec<&str> = NET_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_net_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            // Resolver-backed (plan-72-BB): the harness skips return-type and
            // implementation-name parity — the return is checked through the
            // resolver samples below, the per-name rewrite by
            // `implementation_name_to_url_only`.
            return_type_name: &call_return_type_name,
            // `"or"`-phrased and joined-string shapes; kept hand-authored and
            // checked separately.
            expected_arguments: None,
            param_name_overloads: Some(&|name| {
                call_param_name_overloads(name)
                    .map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            }),
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: Some(&builtin_type_fields),
        };

        // Return type through the resolver, including `close`'s three-way argument
        // union and `connectTcp`'s `String,Integer` / `Address` overloads.
        let ret = |call, args: &'static [&'static str], r: &'static str| parity::ResolverSample {
            call,
            arg_types: args,
            expected_return: Some(r),
            expected_impl: None,
            expected_padding: None,
            expected_type: None,
            expected_overload_target: None,
        };
        let samples = [
            ret(CONNECT_TCP, &["String", "Integer"], SOCKET_TYPE),
            ret(CONNECT_TCP, &[ADDRESS_TYPE], SOCKET_TYPE),
            ret(CLOSE, &[SOCKET_TYPE], "Nothing"),
            ret(CLOSE, &[LISTENER_TYPE], "Nothing"),
            ret(CLOSE, &[UDP_SOCKET_TYPE], "Nothing"),
        ];
        let mut probe = calls.clone();
        probe.push("net.bogus");
        parity::assert_parity(&NET, &probe, &legacy, &samples);
    }
}
