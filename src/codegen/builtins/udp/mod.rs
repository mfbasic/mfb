//! The built-in `udp` package — datagram sockets (plan-110-C).
//!
//! `udp` owns the connectionless half of what `net` used to carry: binding a
//! datagram socket, sending to an address, receiving with the sender's address
//! attached, readiness, and per-direction timeouts. `net` keeps DNS, URL parsing
//! and ICMP; `tcp` took the stream; `tls` takes the encrypted stream.
//!
//! One resource, `Socket`, and one value record, `Datagram`. As in `tcp`,
//! endpoints are `net::Address` rather than a udp-local copy, so an address from
//! `net::lookup` or from a received datagram's `from` field can be sent to
//! directly — and, as in `tcp`, a file that *uses* those addresses must
//! `IMPORT net` too, because imports are not transitive.
//!
//! ## What deliberately did not come across from `net`
//!
//! `net` had `receiveTextFrom` returning a `DatagramText { from, value }`. Neither
//! survives. A datagram is a bounded blob of bytes and the network says nothing
//! about its encoding; decoding on receipt would either have to guess or raise on
//! perfectly valid binary traffic. `receive` therefore always yields bytes, and
//! `encoding::utf8Decode` does the decoding when the caller knows the payload is
//! text. Sending is not symmetric because it has no such ambiguity — the caller
//! already knows what they have — so `send` keeps a `String` overload.
//!
//! ## Relationship to the `net` implementation during the migration
//!
//! Like `tcp`, the members lower through the emitters still living in
//! `net::{gen_shared, gen_io, gen_poll}`; they are moved into this directory by
//! plan-110-E, when net's datagram descriptors are deleted. See plan-110-B §C2.

use crate::codegen::registry::{
    Body, DefaultValue, Parameter, RecordProp, Registry, RegistryPackage, RegistryRecord,
    RegistryResource,
};
use crate::types::ParameterType;

/// A required `udp` member parameter.
pub(crate) fn req(
    name: &'static str,
    desc: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

/// An optional trailing parameter that widens the arity but is NOT default-padded
/// by the registry — the code layer injects the sentinel in `builder_values`.
pub(crate) fn opt(name: &'static str, desc: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty,
        default: DefaultValue::Optional,
    }
}

mod func_bind;
mod func_close;
mod func_local_address;
mod func_poll;
mod func_receive;
mod func_send;
mod func_set_read_timeout;
mod func_set_write_timeout;
pub(crate) mod gen_io;

/// The bare type names — the identity *within* the `udp` package.
pub(crate) const SOCKET_TYPE: &str = "Socket";
pub(crate) const DATAGRAM_TYPE: &str = "Datagram";

/// The package-qualified resource identity, deliberately distinct from
/// `net.UdpSocket` so the two cannot be substituted while both exist.
pub(crate) const SOCKET_TYPE_ID: &str = "udp.Socket";

/// `net::Address` as this package refers to it — the shared record, not a copy.
pub(crate) fn address() -> ParameterType {
    ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE)
}

pub(crate) fn socket() -> ParameterType {
    ParameterType::named(SOCKET_TYPE_ID)
}

const MODULE_INTRO: &str =
    r#"UDP datagram sockets: bind, send, and receive with the sender's address"#;

const MODULE_DESC: &str = r#"The `udp` package exchanges datagrams. `udp::bind` opens a socket on a local
port, `udp::send` addresses one datagram to a peer, and `udp::receive` returns
the next datagram together with the address it came from.

UDP is not a stream and this package does not pretend otherwise. Datagram
boundaries are preserved exactly: one `send` becomes one `receive`, never split
and never merged with its neighbours. There is no connection, no ordering
guarantee, no retransmission, and no delivery confirmation — a datagram that is
lost is simply never received, and a successful `send` means only that the local
OS accepted it for sending.

A zero-length datagram is ordinary and is received as `bytes` of length 0. It does
not mean end-of-stream: UDP has no such concept, and reading one is not an error.

`receive` always returns bytes. Unlike a stream read there is no risk of splitting
a character in half — a datagram arrives whole or not at all — but the network
still says nothing about the payload's encoding, so decoding is the caller's
decision: use `encoding::utf8Decode` when the payload is known to be text.
`udp::send` does accept a `String` directly, because the sender always knows what
it is sending.

Endpoints are `net::Address` values, so an address from `net::lookup` or from a
received datagram's `from` field can be sent to directly. **A program that uses
those addresses must `IMPORT net` as well as `udp`**: imports are not transitive
and a package cannot re-export another's types.

The `Socket` handle is an opaque, owned resource closed automatically by lexical
drop; `udp::close` releases it earlier. For connection-oriented byte streams use
`tcp`."#;

/// Build a native `udp.*` member's body plus any code-form `os_aliases`
/// (`sendText`, `pollList`), distinguished inside the body off `AbiCtx::call`.
pub(crate) fn native_body(
    lower: crate::codegen::registry::AbiFunction,
    os_aliases: &'static [&'static str],
) -> Body {
    Body::abi_function_aliased(lower, os_aliases)
}

/// Register the `udp` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("udp", MODULE_INTRO, MODULE_DESC);

    // `net` for the shared `Address` record every endpoint and every received
    // datagram uses.
    pkg.add_imports(vec!["net"]);

    // The one value record. Field order is ABI-relevant: the native receive
    // emitter writes these two slots in this order.
    pkg.add_record(RegistryRecord {
        name: DATAGRAM_TYPE,
        export: true,
        description: "One received datagram: the payload bytes and the address they came from. A plain, copyable value record.",
        props: vec![
            RecordProp {
                name: "from",
                ty: ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE),
                description: "The address the datagram was sent from. Reply by passing it straight back to `udp::send`.",
            },
            RecordProp {
                name: "bytes",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "The payload, exactly as sent. Empty for a zero-length datagram, which is ordinary and not an end-of-stream.",
            },
        ],
    });

    pkg.add_resource(RegistryResource {
        name: SOCKET_TYPE,
        export: true,
        description: "A bound UDP datagram socket from `udp::bind`, \
                      closed automatically when it leaves scope.",
        close_function: "udp.close",
        sendable: true,
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_bind::register(&mut pkg);
    func_send::register(&mut pkg);
    func_receive::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_close::register(&mut pkg);
    func_local_address::register(&mut pkg);
    func_set_read_timeout::register(&mut pkg);
    func_set_write_timeout::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;

    #[test]
    fn udp_socket_is_qualified_and_distinct_from_net() {
        assert_eq!(
            registry().qualified_builtin_type(super::SOCKET_TYPE_ID),
            Some(super::SOCKET_TYPE.to_string())
        );
        // Routes to udp's own close op, not net's — otherwise a udp socket would
        // be dropped through the wrong package's operation.
        assert_eq!(
            crate::codegen::resource::builtin_resource_close_function(super::SOCKET_TYPE_ID),
            Some("udp.close")
        );
        // net's datagram socket is GONE. Unlike `tcp`, `udp` could not coexist with
        // the surface it replaces: two packages cannot both declare a record named
        // `Datagram`, and `udp` pulls net's injected source in regardless of what
        // the program imports, so plan-110-C had to remove net's datagram surface
        // rather than defer it to plan-110-E (§C1 of that letter).
        assert!(
            !crate::codegen::resource::is_builtin_resource_type("net.UdpSocket"),
            "net.UdpSocket must be gone -- udp.Socket replaces it"
        );
        // plan-110-E has since removed net's stream surface too, so neither half
        // of what `net` used to carry is left.
        assert!(!crate::codegen::resource::is_builtin_resource_type(
            "net.Socket"
        ));
        assert!(!crate::codegen::resource::is_builtin_resource_type(
            "net.Listener"
        ));
    }

    /// The receive emitter writes the `Datagram` record's two slots positionally,
    /// so the declared field order IS the memory layout. Reordering these — or
    /// inserting a field — would silently mis-assign both values.
    #[test]
    fn datagram_field_order_is_from_then_bytes() {
        let pkg = registry().resolve_package("udp").expect("udp package");
        let datagram = pkg
            .records()
            .iter()
            .find(|r| r.name == super::DATAGRAM_TYPE)
            .expect("Datagram record");
        let names: Vec<&str> = datagram.props.iter().map(|p| p.name).collect();
        assert_eq!(names, ["from", "bytes"]);
        // `from` is net's shared Address, not a udp-local copy.
        assert_eq!(datagram.props[0].ty, super::address());
        assert_eq!(
            datagram.props[1].ty,
            crate::types::ParameterType::list_of(crate::types::ParameterType::Byte)
        );
    }

    /// The text-receive shape `net` had is deliberately absent: a datagram's
    /// encoding is not something the network reports, so decoding on receipt would
    /// have to guess or reject valid binary traffic.
    #[test]
    fn no_text_receive_shape_survives() {
        let pkg = registry().resolve_package("udp").expect("udp package");
        assert!(
            pkg.functions().iter().all(|f| f.name != "receiveText"),
            "udp must not carry a text-receive member"
        );
        assert!(
            pkg.records().iter().all(|r| r.name != "DatagramText"),
            "udp must not carry a DatagramText record"
        );
        // Sending text IS supported, as a second overload of `send`.
        let send = pkg
            .functions()
            .iter()
            .find(|f| f.name == "send")
            .expect("send member");
        assert!(
            send.implementations
                .iter()
                .any(|i| i.params[2].ty == crate::types::ParameterType::String),
            "send must accept a String payload"
        );
    }
}
