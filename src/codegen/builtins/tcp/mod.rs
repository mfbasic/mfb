//! The built-in `tcp` package — plaintext TCP streams and listeners (plan-110-B).
//!
//! `tcp` owns the connection-oriented half of what `net` used to carry: connecting
//! out, listening and accepting, transferring bytes, readiness, endpoint queries,
//! and per-direction timeouts. `net` keeps DNS, URL parsing, and ICMP; `udp` takes
//! datagrams (plan-110-C); `tls` takes the encrypted stream (plan-110-D).
//!
//! Two resources: `Socket` (a connected stream) and `Listener` (a bound server
//! endpoint). Both are opaque handles released when their bindings go out of scope, and both
//! share the public `tcp.close` close op — the same shape `net` used, because the
//! runtime record is unchanged. Both are thread-sendable (bug-464): each record is
//! the canonical header alone — fd @8, closed @16 — so the thread-transfer copy
//! carries all of it, and a server can bind on one thread and serve on another.
//!
//! **`Address` is not duplicated here.** Endpoints are `net::Address`, so a value
//! from `net::lookup` feeds `tcp::connect` directly and no conversion exists to get
//! wrong. That cross-package type reference is the reason `tcp` imports `net`.
//!
//! ## Relationship to the `net` implementation during the migration
//!
//! Every member lowers through the emitters that still live in
//! `net::{gen_shared, gen_io, gen_poll}`. Those emitters are parameterized by
//! nothing that names `net`: they marshal file descriptors and `sockaddr`s, and the
//! resource identity is decided by the *descriptor* that calls them. So `tcp`'s
//! members produce byte-identical syscall sequences under `tcp`-owned symbols and
//! `tcp`-owned resource types, with no unsafe cross-identity substitution.
//!
//! The emitters are shared rather than copied, and are physically moved into this
//! directory by plan-110-E, when `net`'s transport descriptors are deleted and
//! nothing else references them. Moving them in this letter would mean editing the
//! same 2,000 lines twice for no intermediate benefit — see this letter's
//! Corrections.
//!
//! ## Surface changes against the old `net` spelling
//!
//! - `connectTcp` → `connect`, `listenTcp` → `listen` (the package name already
//!   says TCP).
//! - `readText`/`writeText` are **gone**: `write` takes a `String` as a second
//!   overload, and `read` returns bytes only. Decoding is `encoding`'s job, and a
//!   stream read cannot promise to stop on a character boundary anyway.

use crate::codegen::registry::{
    Body, DefaultValue, Parameter, Registry, RegistryPackage, RegistryResource,
};
use crate::types::ParameterType;

/// A required `tcp` member parameter.
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
/// by the registry — the code layer injects the timeout/backlog sentinel in
/// `builder_values`, exactly as it does for the `net` originals.
pub(crate) fn opt(name: &'static str, desc: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty,
        default: DefaultValue::Optional,
    }
}

mod func_accept;
mod func_close;
mod func_connect;
mod func_listen;
mod func_local_address;
mod func_poll;
mod func_read;
mod func_remote_address;
mod func_set_read_timeout;
mod func_set_write_timeout;
mod func_write;
pub(crate) mod gen_io;

/// The bare resource type names — the identity *within* the `tcp` package.
pub(crate) const SOCKET_TYPE: &str = "Socket";
pub(crate) const LISTENER_TYPE: &str = "Listener";

/// The package-qualified resource identities. Every `RES` binding, parameter, and
/// return of a tcp resource carries one of these, and they are the `ResourceRegistry`
/// keys — deliberately distinct from `net.Socket`/`net.Listener` so the two cannot
/// be substituted for each other while both exist.
pub(crate) const SOCKET_TYPE_ID: &str = "tcp.Socket";
pub(crate) const LISTENER_TYPE_ID: &str = "tcp.Listener";

/// `net::Address` as this package refers to it. Endpoints are the shared `net`
/// record, not a tcp-local copy.
pub(crate) fn address() -> ParameterType {
    ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE)
}

pub(crate) fn socket() -> ParameterType {
    ParameterType::named(SOCKET_TYPE_ID)
}

pub(crate) fn listener() -> ParameterType {
    ParameterType::named(LISTENER_TYPE_ID)
}

const MODULE_INTRO: &str =
    r#"Plaintext TCP client connections, server listeners, and byte streams"#;

const MODULE_DESC: &str = r#"The `tcp` package opens and accepts plaintext TCP connections and moves bytes
over them. `tcp::connect` opens an outbound stream; `tcp::listen` and
`tcp::accept` run a server; `tcp::read` and `tcp::write` transfer bytes;
`tcp::poll` waits for readability, over one socket or a list of them.

Endpoints are `net::Address` values, so an address resolved by `net::lookup` can
be handed straight to `tcp::connect`, and `tcp::localAddress` /
`tcp::remoteAddress` report the same shape back.

**A program that uses those addresses must `IMPORT net` as well as `tcp`.**
Imports are not transitive and a package cannot re-export another's types (see
`mfb spec language modules-and-packages`), so `Address` is only nameable in a
file that imports the package declaring it. Without that import, the value
returned by `tcp::localAddress` has no nameable type and the *next* call using it
fails to resolve. Only the address-valued members are affected: `tcp::connect`,
`tcp::listen`, `tcp::read`, and `tcp::write` need nothing but `IMPORT tcp`.

`Socket` and `Listener` are opaque handles that close themselves when their
binding goes out of scope. `tcp::close` closes one earlier — to release a
listening port for reuse, to let a peer observe the end of the stream promptly,
or to bound how many connections a long-running program holds open at once.

`tcp::read` returns bytes and never text: a stream read stops wherever the
network divided it, which need not be a character boundary, so decoding belongs
to `encoding` once a whole message has been assembled. `tcp::write` does accept a
`String` directly as a second overload and sends its UTF-8 bytes.

For encrypted connections use `tls`; for datagrams use `udp`; for name resolution
and URL parsing use `net`."#;

/// Build a native `tcp.*` member's body: the member's own lowering plus any
/// code-form `os_aliases` the overload set emits (`connectAddr`, `pollList`),
/// distinguished inside the body off `AbiCtx::call`.
pub(crate) fn native_body(
    lower: crate::codegen::registry::AbiFunction,
    os_aliases: &'static [&'static str],
) -> Body {
    Body::abi_function_aliased(lower, os_aliases)
}

/// Register the `tcp` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("tcp", MODULE_INTRO, MODULE_DESC);

    // `net` for the shared `Address` record that every endpoint uses.
    pkg.add_imports(vec!["net"]);

    pkg.add_resource(RegistryResource {
        name: SOCKET_TYPE,
        export: true,
        description: "A connected TCP stream from `tcp::connect` or `tcp::accept`, \
                      closed automatically when its binding goes out of scope.",
        close_function: "tcp.close",
        sendable: true,
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });
    pkg.add_resource(RegistryResource {
        name: LISTENER_TYPE,
        export: true,
        description: "A bound TCP server endpoint from `tcp::listen`; \
                      `tcp::accept` draws connections from it.",
        close_function: "tcp.close",
        // Thread-sendable (bug-464): a listener's record is the canonical header
        // and nothing else -- the listening fd @8 and the closed flag @16, the
        // same shape as the `Socket` above -- so the thread-transfer copy already
        // carries all of it. The earlier `false` was policy alone ("a listener
        // accepts on its owning thread"), which ruled out the ordinary server
        // shape of binding on one thread and serving on another.
        sendable: true,
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_connect::register(&mut pkg);
    func_listen::register(&mut pkg);
    func_accept::register(&mut pkg);
    func_read::register(&mut pkg);
    func_write::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_close::register(&mut pkg);
    func_local_address::register(&mut pkg);
    func_remote_address::register(&mut pkg);
    func_set_read_timeout::register(&mut pkg);
    func_set_write_timeout::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;

    #[test]
    fn tcp_resources_are_qualified_and_distinct_from_net() {
        // Qualified lookup resolves both handles...
        assert_eq!(
            registry().qualified_builtin_type(super::SOCKET_TYPE_ID),
            Some(super::SOCKET_TYPE.to_string())
        );
        assert_eq!(
            registry().qualified_builtin_type(super::LISTENER_TYPE_ID),
            Some(super::LISTENER_TYPE.to_string())
        );
        // ...and they are recognized as builtin resources with tcp's own close op,
        // not net's. If these ever came back as `net.close`, a tcp socket would be
        // dropped through the wrong package's op.
        assert_eq!(
            crate::codegen::resource::builtin_resource_close_function(
                &crate::types::ParameterType::declared(super::SOCKET_TYPE_ID)
            ),
            Some("tcp.close")
        );
        assert_eq!(
            crate::codegen::resource::builtin_resource_close_function(
                &crate::types::ParameterType::declared(super::LISTENER_TYPE_ID)
            ),
            Some("tcp.close")
        );
        // plan-110-E removed net's transport surface, so the identity that this
        // one was "distinct from" no longer exists at all -- which is a stronger
        // guarantee than the two coexisting and not being substitutable.
        assert!(!crate::codegen::resource::is_builtin_resource_type(
            &crate::types::ParameterType::declared("net.Socket")
        ));
        assert!(!crate::codegen::resource::is_builtin_resource_type(
            &crate::types::ParameterType::declared("net.Listener")
        ));
    }

    #[test]
    fn tcp_socket_and_listener_are_both_sendable() {
        // bug-464: both cross a thread boundary. This test asserted
        // `!sendable` for the Listener until then -- carried into plan-110-B
        // (008d745c2) from plan-03-net.md §4.4's v1 deferral, which was a
        // product decision ("a listener accepts on its owning thread") and not a
        // safety invariant. The Listener record is the canonical header alone
        // (fd @8, closed @16), the same shape as the Socket, so the
        // thread-transfer copy already carried all of it; the restriction only
        // ruled out binding on one thread and serving on another.
        // `tests/rt-behavior/threads/thread-transfer-tcp-listener-rt` proves the
        // new contract at runtime by accepting a real connection on a
        // transferred listener.
        assert!(crate::codegen::resource::is_builtin_sendable_resource_type(
            &crate::types::ParameterType::declared(super::SOCKET_TYPE_ID)
        ));
        assert!(crate::codegen::resource::is_builtin_sendable_resource_type(
            &crate::types::ParameterType::declared(super::LISTENER_TYPE_ID)
        ));
    }

    #[test]
    fn tcp_endpoints_use_the_shared_net_address_record() {
        // Not a tcp-local copy: a `net::lookup` result must feed `tcp::connect`
        // with no conversion, so the parameter type has to be net's record.
        let pkg = registry().resolve_package("tcp").expect("tcp package");
        assert!(pkg.records().is_empty(), "tcp declares no value records");
        let connect = pkg
            .functions()
            .iter()
            .find(|f| f.name == "connect")
            .expect("connect member");
        assert!(
            connect
                .implementations
                .iter()
                .any(|i| i.params[0].ty == super::address()),
            "one connect overload must take a net::Address"
        );
    }
}
