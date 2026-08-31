//! The built-in `net` package (DNS, ICMP, addresses, and URLs) on the clean-room
//! registry.
//!
//! `net` names hosts; it does not connect to them. `net::lookup` resolves a host
//! name and `net::ping` sends one ICMP echo request. plan-110 moved every
//! transport out: `tcp` took the stream (`Socket`/`Listener`), `udp` the datagram
//! (`Socket`/`Datagram`), and `tls` the encrypted stream. **`net` owns no
//! resources at all.**
//!
//! What all four still share is the `Address` value record, which stays here: it
//! is what `net::lookup` returns, what a `udp` datagram reports as its sender, and
//! what every transport's connect/bind/localAddress speaks. A file that names an
//! `Address` must `IMPORT net` as well as its transport — imports are not
//! transitive.
//!
//! Like `fs`/`audio`, `net` is a **native OS-seam** package on the
//! `Body::abi_function` clean-room shape: `lookup` and `ping` each carry a
//! per-platform runtime-helper lowering. The syscall emission lives in `gen_io`
//! (the resolver) and `gen_ping`, over the shared address/handle primitives in
//! [`crate::codegen::os::socket`]; each member owns its own
//! [`Body::abi_function_aliased`] body in its `func_*.rs` (`lower_<name>`) and
//! finalizes, selecting the posix/win emission by `platform.family()`. `ping`'s
//! `pingAddr` code-form alias routes through `is_abi_function_call` /
//! `abi_function_lower` (the aux→primary routing is registry data), and the `Net`
//! runtime family is preserved via `abi_function_family` so the `_mfb_rt_net_*`
//! symbols are unchanged.
//!
//! The pure URL string work (`toUrl`/`percentDecode`/`parseQuery`, the `Url` value
//! record, and the `toString(net::Url)` renderer) is source-backed: the three
//! members carry their `__net_*` bodies as `Body::mfb` in their `func_*.rs`, the
//! private helpers live one per `helper_*.rs` (`add_helper` — private-only), and
//! `toString(net::Url)` is registered as an [`RegistryPackage::add_override`].
//!
//! `lookup`/`ping` are argument-shape-overloaded — two/four `Implementation`s — so
//! the registry's generic overload/return resolution answers everything with no
//! custom resolver (the datetime/net idiom).

use crate::codegen::registry::{
    Body, DefaultValue, EnumVariant, Parameter, RecordProp, Registry, RegistryEnum,
    RegistryOverride, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

/// A required `net` member parameter. Docs live in the `src/docs/man/builtins/net`
/// pages (as the legacy descriptor's empty doc strings did); the registry carries
/// name/aliases/type for arity, coercion, and named-argument binding.
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
/// by the registry — the code layer injects the timeout/port/backlog sentinel
/// (`builder_values`), matching the legacy `DefaultValue::Optional`.
pub(crate) fn opt(name: &'static str, desc: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty,
        default: DefaultValue::Optional,
    }
}

/// The qualified socket / listener resource types as `ParameterType`s.
mod func_lookup;
mod func_parse_query;
mod func_percent_decode;
mod func_ping;
mod func_to_url;

mod helper_authority_end;
mod helper_decode_query_component;
mod helper_default_port;
mod helper_index_of;
mod helper_last_index_of;
mod helper_parse_port;
mod helper_percent_decode_impl;
mod helper_slice;
mod helper_url_to_string;

pub(crate) mod gen_io;
mod gen_ping;

/// The bare resource/record type names — the identity *within* the `net` package
/// (the `RegistryResource`/`RegistryRecord` name, the `type` half of a qualified
/// id). Used for registry-internal lookups and by the code layer.
pub(crate) const ADDRESS_TYPE: &str = "Address";
/// The `PingStatus` enum and `PingResult` record `net::ping` reports through
/// (plan-110-A). `PingStatus`'s variant ORDER is its wire contract: a variant's
/// ordinal is its declaration index, and `gen_ping` emits those ordinals directly.
pub(crate) const PING_STATUS_TYPE: &str = "PingStatus";
pub(crate) const PING_RESULT_TYPE: &str = "PingResult";
/// The `Url` value record's name — registry-modeled (`add_record`, DOC
/// round-tripped via `description`).
pub(crate) const URL_TYPE: &str = "Url";

/// The internal source-companion (`package.mfb`) render target for the
/// `toString(net::Url)` override — routed here by [`RegistryPackage::add_override`].
pub(crate) const URL_TO_STRING: &str = "__net_urlToString";

const MODULE_INTRO: &str = r#"DNS lookup, ICMP echo, URL parsing, and the shared network address"#;
const MODULE_DESC: &str = r#"The `net` package names hosts. `net::lookup` resolves a host name to a list of
`Address` values, and `net::ping` sends one ICMP echo request and reports how the
host answered. Nothing in this package opens a connection.

`Address` is the shared endpoint record every transport speaks: an address from
`net::lookup`, from a received datagram's `from` field, or from a socket's local
or remote address query can be handed straight to any of them. **A program that
names an `Address` must `IMPORT net` as well as its transport** — imports are not
transitive and a package cannot re-export another's types.

The transports live in their own packages: `tcp` for byte streams, `udp` for
datagrams, `tls` for encrypted streams, and `http` for requests and responses.

The package also parses and renders URLs: `net::toUrl` decomposes an absolute URL
into a `Url` value record, `toString` renders it back, and `net::percentDecode` /
`net::parseQuery` decode request-target components.

`net` has no handles to open or close — every call takes and returns ordinary values."#;

/// Build a native `net.*` member's `Body::abi_function_aliased`: its own per-member
/// lowering `lower` (which lives in the member's `func_*.rs` and calls the matching
/// `gen_io`/`gen_ping` emitter over `codegen::os::socket`), plus any code-form
/// `os_aliases` (`pingAddr`) the overload emits — the `abi_function` successor to
/// the `native_os_seam` twin idiom (crypto/io/fs/audio shape). Aliases route to the
/// same member body through `abi_function_lower`, distinguished off `AbiCtx::call`.
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

    // The `Url` value record (its DOC block round-trips via `description`).
    pkg.add_record(RegistryRecord {
        name: URL_TYPE,
        export: true,
        description: "A parsed URL, produced by `net::toUrl` and rendered back with `net::toString`. Each component is stored decomposed for direct access.",
        props: vec![
            RecordProp {
                name: "scheme",
                ty: ParameterType::String,
                description: "The scheme, lowercased (e.g. `\"http\"` or `\"https\"`).",
            },
            RecordProp {
                name: "username",
                ty: ParameterType::String,
                description: "The userinfo before the `:`, or `\"\"` if none.",
            },
            RecordProp {
                name: "password",
                ty: ParameterType::String,
                description: "The userinfo after the `:`, or `\"\"` if none.",
            },
            RecordProp {
                name: "host",
                ty: ParameterType::String,
                description: "The registered name or IP literal (an IPv6 literal without brackets).",
            },
            RecordProp {
                name: "port",
                ty: ParameterType::Integer,
                description: "The explicit port, or the scheme default (80 for http, 443 for https).",
            },
            RecordProp {
                name: "path",
                ty: ParameterType::String,
                description: "The path, beginning with `/`; `/` when the href had none.",
            },
            RecordProp {
                name: "query",
                ty: ParameterType::String,
                description: "The raw query without the leading `?`, or `\"\"` if none.",
            },
            RecordProp {
                name: "fragment",
                ty: ParameterType::String,
                description: "The raw fragment without the leading `#`, or `\"\"` if none.",
            },
        ],
    });

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

    // `net::ping`'s two value types (plan-110-A). The enum's variant ORDER is the
    // contract, not just documentation: a variant's ordinal is its declaration
    // index, and `gen_ping` writes those ordinals into `PingResult.status`
    // directly, so reordering these silently changes what a ping reports.
    pkg.add_enum(RegistryEnum {
        name: PING_STATUS_TYPE,
        export: true,
        variants: vec![
            EnumVariant {
                name: "Ok",
                description: "An echo reply came back; `rttMs`, `ttl`, and `size` are measured.",
                advisory: None,
            },
            EnumVariant {
                name: "Timeout",
                description: "No reply arrived before the deadline.",
                advisory: None,
            },
            EnumVariant {
                name: "Unreachable",
                description: "A router reported the destination unreachable.",
                advisory: None,
            },
            EnumVariant {
                name: "TtlExceeded",
                description: "The request outlived its TTL in transit and a router reported it.",
                advisory: None,
            },
        ],
    });
    // Field ORDER is likewise contract: `gen_ping` builds this record by writing
    // five consecutive 8-byte slots at the offsets these declarations fix.
    pkg.add_record(RegistryRecord {
        name: PING_RESULT_TYPE,
        export: true,
        description: "The outcome of one `net::ping`. Only an `Ok` status carries measured `rttMs`, `ttl`, and `size`; every other status zeroes all three.",
        props: vec![
            RecordProp {
                name: "status",
                ty: ParameterType::named(PING_STATUS_TYPE),
                description: "How the host answered.",
            },
            RecordProp {
                name: "address",
                ty: ParameterType::named(ADDRESS_TYPE),
                description: "Who answered — the responder for `Ok`, the reporting router for `Unreachable`/`TtlExceeded`, and the destination that was aimed at for `Timeout`. Its `port` is always `0`: ICMP has no transport port.",
            },
            RecordProp {
                name: "rttMs",
                // Float, not Integer: a loopback round trip is tens of
                // microseconds and would always truncate to 0 (plan-110-A §C3).
                ty: ParameterType::Float,
                description: "The round-trip time in milliseconds, or `0.0` when the status is not `Ok`.",
            },
            RecordProp {
                name: "ttl",
                ty: ParameterType::Integer,
                description: "The TTL observed on the reply, or `0` when the status is not `Ok`.",
            },
            RecordProp {
                name: "size",
                ty: ParameterType::Integer,
                description: "The number of payload bytes echoed back, or `0` when the status is not `Ok`.",
            },
        ],
    });

    // plan-110-E: net has NO resources any more. Its stream handles (`Socket`,
    // `Listener`) moved to `tcp`, and `UdpSocket` moved to `udp` in plan-110-C.
    // What is left -- resolution and URLs -- is all values, so nothing net
    // returns needs a close op or a lexical drop.
    // `toString(net::Url)` renders a `Url` back to an absolute href — the registry
    // home of the hand row in `builtins::general_override_target`.
    pkg.add_override(RegistryOverride {
        builtin: "toString",
        arg_type: URL_TYPE,
        helper: URL_TO_STRING,
    });

    // The shared private `__net_*` URL-string helpers, one `helper_*.rs` per FUNC
    // (`add_helper` — private-only; `__net_urlToString` is the `toString(Url)`
    // override target above, reached through `add_override`, not a function). The
    // three public source members' bodies ride their `func_*.rs` descriptors as
    // `Body::mfb`.
    helper_index_of::register(&mut pkg);
    helper_last_index_of::register(&mut pkg);
    helper_slice::register(&mut pkg);
    helper_default_port::register(&mut pkg);
    helper_authority_end::register(&mut pkg);
    helper_parse_port::register(&mut pkg);
    helper_url_to_string::register(&mut pkg);
    helper_percent_decode_impl::register(&mut pkg);
    helper_decode_query_component::register(&mut pkg);

    func_lookup::register(&mut pkg);
    func_ping::register(&mut pkg);
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
    crate::codegen::registry::inject_late_pass(ast, "net", SOURCE_LABEL, SOURCE_DOC)
}

/// The same injection onto the elaborated project the former source checker consumes
/// (plan-106-D).
#[cfg(test)] // the HIR-domain chain serves the in-process tests only (plan-107-D)
pub(crate) fn augmented_hir_project(
    hir: &crate::hir::HirProject,
) -> Result<crate::hir::HirProject, ()> {
    crate::codegen::registry::inject_late_pass_hir(hir, "net", SOURCE_LABEL, SOURCE_DOC)
}
