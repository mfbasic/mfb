//! The built-in `net` package (DNS, TCP, and UDP sockets) on the clean-room
//! registry.
//!
//! `net` resolves host names, opens and accepts plaintext TCP connections, binds
//! UDP datagram sockets, and transfers bytes over both. Its three resources —
//! `Socket` (a connected TCP stream), `Listener` (a bound TCP server endpoint),
//! and `UdpSocket` (a bound datagram socket) — are opaque, owned handles released
//! by lexical drop.
//!
//! Like `fs`/`audio`, `net` is a **native OS-seam** package migrated to the
//! `Body::abi_function` clean-room shape: every socket/DNS/UDP member carries a
//! per-platform runtime-helper lowering. The relocated syscall emission lives in the
//! `gen_shared`/`gen_io`/`gen_poll` `lower_net_*_helper` emitters; each member owns
//! its own [`Body::abi_function_aliased`] body in its `func_*.rs` (`lower_<name>`),
//! which calls the matching `lower_net_*_helper` (with any bool/alias discriminant)
//! and finalizes; each emitter selects the posix/win emission by `platform.family()`.
//! The member plus its `connectTcpAddr` /
//! `pollList` code-form aliases route through `is_abi_function_call` /
//! `abi_function_lower` (the aux→primary routing is registry data), and the `Net`
//! runtime family is preserved via `abi_function_family` so the `_mfb_rt_net_*`
//! symbols are unchanged.
//!
//! The pure URL string work (`toUrl`/`percentDecode`/`parseQuery`, the `Url` value
//! record, and the `toString(net::Url)` renderer) is source-backed
//! (`package.mfb`): those three members are `Body::Rewrite` targets, and
//! `toString(net::Url)` is registered as an [`RegistryPackage::add_override`].
//!
//! `poll`/`connectTcp` are argument-shape-overloaded — two/four `Implementation`s —
//! so the registry's generic overload/return resolution answers everything with no
//! custom resolver (the datetime/net idiom).

use crate::codegen::registry::{
    Body, DefaultValue, Parameter, RecordProp, Registry, RegistryOverride, RegistryPackage,
    RegistryRecord, RegistryResource,
};
use crate::types::ParameterType;

/// A required `net` member parameter. Docs live in the `src/docs/man/builtins/net`
/// pages (as the legacy descriptor's empty doc strings did); the registry carries
/// name/aliases/type for arity, coercion, and named-argument binding.
pub(crate) fn req(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

/// An optional trailing parameter that widens the arity but is NOT default-padded
/// by the registry — the code layer injects the timeout/port/backlog sentinel
/// (`builder_values`), matching the legacy `DefaultValue::Optional`.
pub(crate) fn opt(name: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::Optional,
    }
}

/// The qualified socket / listener / UDP resource types as `ParameterType`s.
pub(crate) fn socket() -> ParameterType {
    ParameterType::named(SOCKET_TYPE_ID)
}
pub(crate) fn listener() -> ParameterType {
    ParameterType::named(LISTENER_TYPE_ID)
}
pub(crate) fn udp() -> ParameterType {
    ParameterType::named(UDP_SOCKET_TYPE_ID)
}

mod func_accept;
mod func_bind_udp;
mod func_close;
mod func_connect_tcp;
mod func_listen_tcp;
mod func_local_address;
mod func_lookup;
mod func_parse_query;
mod func_percent_decode;
mod func_poll;
mod func_read;
mod func_read_text;
mod func_receive_from;
mod func_receive_text_from;
mod func_remote_address;
mod func_send_text_to;
mod func_send_to;
mod func_set_read_timeout;
mod func_set_write_timeout;
mod func_to_url;
mod func_write;
mod func_write_text;

mod gen_io;
mod gen_poll;
pub(crate) mod gen_shared;

/// The bare resource/record type names — the identity *within* the `net` package
/// (the `RegistryResource`/`RegistryRecord` name, the `type` half of a qualified
/// id). Used for registry-internal lookups and by the code layer.
pub(crate) const SOCKET_TYPE: &str = "Socket";
pub(crate) const LISTENER_TYPE: &str = "Listener";
pub(crate) const UDP_SOCKET_TYPE: &str = "UdpSocket";
pub(crate) const ADDRESS_TYPE: &str = "Address";
pub(crate) const DATAGRAM_TYPE: &str = "Datagram";
pub(crate) const DATAGRAM_TEXT_TYPE: &str = "DatagramText";
/// The `Url` value record's name — declared in `package.mfb` (with its `DOC` block)
/// and registered as a source type.
pub(crate) const URL_TYPE: &str = "Url";

/// The package-qualified type identities (`net.Socket`, `net.Listener`,
/// `net.UdpSocket`) — plan-97 / bug-441. The string every `RES` binding,
/// parameter, and return of a net resource carries; the `ResourceRegistry` key.
pub(crate) const SOCKET_TYPE_ID: &str = "net.Socket";
pub(crate) const LISTENER_TYPE_ID: &str = "net.Listener";
pub(crate) const UDP_SOCKET_TYPE_ID: &str = "net.UdpSocket";

/// The internal source-companion (`package.mfb`) render target for the
/// `toString(net::Url)` override — routed here by [`RegistryPackage::add_override`].
pub(crate) const URL_TO_STRING: &str = "__net_urlToString";

const MODULE_INTRO: &str =
    r#"DNS lookup, TCP client and server sockets, UDP datagram sockets, and URL parsing"#;
const MODULE_DESC: &str = r#"The `net` package resolves host names and opens plaintext TCP and UDP network
connections. `net::connectTcp` opens an outbound TCP stream; `net::listenTcp` and
`net::accept` run a TCP server; `net::bindUdp`, `net::sendTo`, and
`net::receiveFrom` exchange UDP datagrams; and `net::read`/`net::write` (and their
text forms) transfer bytes. For encrypted TLS connections use `tls`; for HTTP use
`http`.

The package also parses and renders URLs: `net::toUrl` decomposes an absolute
URL into a `Url` value record, `toString` renders it back, and
`net::percentDecode` / `net::parseQuery` decode request-target components. The
`Socket`, `Listener`, and `UdpSocket` handles are opaque, owned resources closed
automatically by lexical drop; `net::close` releases one earlier."#;

/// Build a native `net.*` member's `Body::abi_function_aliased`: its own per-member
/// lowering `lower` (which lives in the member's `func_*.rs` and calls the matching
/// `gen_shared`/`gen_io`/`gen_poll` `lower_net_*_helper` emitter), plus any code-form
/// `os_aliases` (`connectTcpAddr`/`pollList`) the overload emits — the `abi_function`
/// successor to the `native_os_seam` twin idiom (crypto/io/fs/audio shape). Aliases
/// route to the same member body through `abi_function_lower`, distinguished off
/// `AbiCtx::call`.
pub(crate) fn native_body(
    lower: crate::codegen::registry::AbiFunction,
    os_aliases: &'static [&'static str],
) -> Body {
    Body::abi_function_aliased(lower, os_aliases)
}

/// Register the `net` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("net", MODULE_INTRO, MODULE_DESC);

    // The URL string helpers are pure `strings`/`collections` source.
    pkg.add_imports(vec!["strings", "collections"]);

    // The `Url` value record is declared (with its DOC block) in `package.mfb`; the
    // registry only records the name so the generic type query recognizes it.
    pkg.add_source_types(&[URL_TYPE]);

    // The value records the native helpers construct: `Address` (localAddress /
    // remoteAddress / a datagram's `from`), and the two datagram shapes. Rendered as
    // `TYPE` declarations into the injected source (the layout the native builders
    // write matches field order).
    pkg.add_record(RegistryRecord {
        name: ADDRESS_TYPE,
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "host",
                ty: ParameterType::String,
                description: "The peer's textual IP address.",
            },
            RecordProp {
                name: "port",
                ty: ParameterType::Integer,
                description: "The peer's port.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: DATAGRAM_TYPE,
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "from",
                ty: ParameterType::named(ADDRESS_TYPE),
                description: "The datagram's source address.",
            },
            RecordProp {
                name: "bytes",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "The datagram payload bytes.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: DATAGRAM_TEXT_TYPE,
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "from",
                ty: ParameterType::named(ADDRESS_TYPE),
                description: "The datagram's source address.",
            },
            RecordProp {
                name: "value",
                ty: ParameterType::String,
                description: "The datagram payload as UTF-8 text.",
            },
        ],
    });

    // The three opaque socket handles. All share the public `net.close` close op;
    // a `Listener` accepts on its owning thread and is not thread-sendable.
    pkg.add_resource(RegistryResource {
        name: SOCKET_TYPE,
        export: true,
        description: "A connected TCP stream from `net::connectTcp` or `net::accept`, \
                      closed automatically when it leaves scope.",
        close_function: "net.close",
        sendable: true,
        close_may_fail: true,
        kind: crate::builtins::resource::ResourceKind::Builtin,
    });
    pkg.add_resource(RegistryResource {
        name: LISTENER_TYPE,
        export: true,
        description: "A bound TCP server endpoint from `net::listenTcp`; \
                      `net::accept` draws connections from it.",
        close_function: "net.close",
        sendable: false,
        close_may_fail: true,
        kind: crate::builtins::resource::ResourceKind::Builtin,
    });
    pkg.add_resource(RegistryResource {
        name: UDP_SOCKET_TYPE,
        export: true,
        description: "A bound UDP datagram socket from `net::bindUdp`.",
        close_function: "net.close",
        sendable: true,
        close_may_fail: true,
        kind: crate::builtins::resource::ResourceKind::Builtin,
    });

    // `toString(net::Url)` renders a `Url` back to an absolute href — the registry
    // home of the hand row in `builtins::general_override_target`.
    pkg.add_override(RegistryOverride {
        builtin: "toString",
        arg_type: URL_TYPE,
        helper: URL_TO_STRING,
    });

    // The shared private `__net_*` helpers, the `Url` TYPE, and the three
    // source-member bodies (`__net_toUrl`/`__net_percentDecode`/`__net_parseQuery`).
    pkg.add_helper(crate::codegen::registry::RegistryHelper::always(
        "net_package",
        include_str!("package.mfb"),
    ));

    func_lookup::register(&mut pkg);
    func_connect_tcp::register(&mut pkg);
    func_listen_tcp::register(&mut pkg);
    func_accept::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_read::register(&mut pkg);
    func_read_text::register(&mut pkg);
    func_write::register(&mut pkg);
    func_write_text::register(&mut pkg);
    func_close::register(&mut pkg);
    func_local_address::register(&mut pkg);
    func_remote_address::register(&mut pkg);
    func_set_read_timeout::register(&mut pkg);
    func_set_write_timeout::register(&mut pkg);
    func_bind_udp::register(&mut pkg);
    func_receive_from::register(&mut pkg);
    func_receive_text_from::register(&mut pkg);
    func_send_to::register(&mut pkg);
    func_send_text_to::register(&mut pkg);
    func_to_url::register(&mut pkg);
    func_percent_decode::register(&mut pkg);
    func_parse_query::register(&mut pkg);

    r.add_package(pkg);
}

/// The synthetic source path/doc labels — kept byte-identical to the legacy
/// `package_source_glue!("net", "<builtin-net>", "builtins/net.mfb", …)` so the
/// injected `.mfb`'s `.ast`/`.ir` loc metadata does not drift.
const SOURCE_LABEL: &str = "<builtin-net>";
const SOURCE_DOC: &str = "builtins/net.mfb";

/// Inject the `net` source companion (`package.mfb`) as a **dedicated late pass**,
/// mirroring `encoding`: `net` is a transitive dependency of `http` (whose injected
/// source `IMPORT net`s), and the generic single-pass `registry::augment_project`
/// (which skips `net`) cannot see that transitive import. Run after `http`'s late
/// pass and before `strings`, so `net::uses_package` sees `http`'s `IMPORT net` and
/// `strings::uses_package` sees `net`'s `IMPORT strings`.
pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    let Some(pkg) = crate::codegen::registry::registry().resolve_package("net") else {
        return Ok(ast.clone());
    };
    if !pkg.is_imported_by(ast) {
        return Ok(ast.clone());
    }
    let file = crate::ast::parse_source_internal(
        std::path::Path::new(SOURCE_LABEL),
        SOURCE_DOC,
        &pkg.get_mfb(),
    )?;
    let mut augmented = ast.clone();
    augmented.files.push(file);
    Ok(augmented)
}
