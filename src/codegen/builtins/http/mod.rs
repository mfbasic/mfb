//! The built-in `http` package (a blocking + non-blocking HTTP/1.1 client and a
//! small HTTP server) on the clean-room registry.
//!
//! Like `csv`/`json`, `http` is a **source package**: every member is pure MFBASIC
//! protocol string work — each public member's `__http_*` body rides its `func_*.rs`
//! descriptor as `Body::mfb`, and the private helpers live one per `helper_*.rs`
//! (`add_helper` — private-only). Every byte on the wire goes through the native
//! `net`/`tls` packages; `http` introduces no new intrinsics, resources, or runtime
//! specs.
//!
//! The value types (`Response`, `Request`, `RequestPart`, `Route`, the non-blocking
//! `Stream` union + its `PendingState`) are registry-modeled (`add_record`/
//! `add_union`, DOC round-tripped via `description`). `handleRequest` is overloaded
//! by listener type (`tcp::Listener` vs `tls::Listener`); that is two
//! `Implementation`s, each rewriting to its own transport body, selected by the
//! generic overload resolution (the datetime/net idiom, no custom resolver).

use crate::codegen::registry::{
    DefaultValue, Parameter, RecordProp, Registry, RegistryPackage, RegistryRecord, RegistryUnion,
    UnionVariant,
};
use crate::types::ParameterType;

mod func_bytes;
mod func_done;
mod func_finish;
mod func_handle_request;
mod func_json;
mod func_ok;
mod func_pump;
mod func_read;
mod func_ready;
mod func_respond_file;
mod func_respond_path;
mod func_response_default;
mod func_route;
mod func_server;
mod func_server_ssl;
mod func_start_read;
mod func_status;
mod func_with_header;
mod func_write;

mod helper_add_part;
mod helper_build_request;
mod helper_build_response;
mod helper_byte_slice;
mod helper_bytes_match_at;
mod helper_bytes_to_text;
mod helper_check_response;
mod helper_chunked_complete;
mod helper_chunked_scan;
mod helper_dec_to_int;
mod helper_dechunk_bytes;
mod helper_decode_body;
mod helper_default_port;
mod helper_dispatch;
mod helper_disposition_param;
mod helper_empty_request;
mod helper_ext_content_type;
mod helper_frame_advance;
mod helper_frame_complete;
mod helper_frame_start;
mod helper_framing_length;
mod helper_has_control_bytes;
mod helper_has_field_control_bytes;
mod helper_header_map_from_head;
mod helper_header_value;
mod helper_hex_to_int;
mod helper_host_header;
mod helper_index_of;
mod helper_index_of_bytes;
mod helper_invoke_handler;
mod helper_is_extra_header;
mod helper_last_index_of;
mod helper_limits;
mod helper_linger_net;
mod helper_linger_tls;
mod helper_match_path;
mod helper_multipart_boundary;
mod helper_normalize_method;
mod helper_normalize_path;
mod helper_parse_multipart;
mod helper_parse_request;
mod helper_parse_response;
mod helper_parse_status_line;
mod helper_part_header;
mod helper_read_net;
mod helper_read_request_net;
mod helper_read_request_tls;
mod helper_read_tls;
mod helper_reason_phrase;
mod helper_request_framing;
mod helper_request_header_map;
mod helper_request_target;
mod helper_response_with;
mod helper_segments;
mod helper_serialize_head;
mod helper_slice;
mod helper_start_exchange;
mod helper_validate_pattern;
mod helper_wait_readable;

/// The registry-modeled value-type names. The `Stream STATE PendingState`
/// composite is the stateful-resource string a non-blocking `Stream` carries.
pub(crate) const RESPONSE_TYPE: &str = "Response";
pub(crate) const REQUEST_TYPE: &str = "Request";
pub(crate) const ROUTE_TYPE: &str = "Route";
/// The stateful `Stream` a non-blocking exchange hands back. Built
/// structurally, not as `named(STREAM_STATE)`: a `ParameterType` that merely
/// *spells* ` STATE ` is an opaque nominal, and since plan-111-A gave `STATE` a
/// variant that nominal reports no state and compares unequal to the same
/// spelling parsed from a source annotation. As `named(...)` this made
/// `RES s AS http::Stream STATE PendingState = http::startRead(u)` fail with
/// `TYPE_BINDING_MISMATCH` ("initializer type Stream STATE PendingState,
/// expected Stream").
pub(crate) fn stream_state() -> ParameterType {
    ParameterType::stateful(
        ParameterType::named("Stream"),
        ParameterType::named("PendingState"),
    )
}
pub(crate) const LISTENER_TYPE: &str = "tcp.Listener";
pub(crate) const TLS_LISTENER_TYPE: &str = "tls.Listener";
pub(crate) const FILE_TYPE: &str = "fs.File";
/// The route handler's exact function type, `FUNC(Request) AS Response`.
///
/// plan-111-F: built as the `Func` variant rather than parsed from a spelling.
/// The STRUCTURE is what matters here — the registry matcher compares
/// element-wise, so a wrong-shaped handler (`FUNC(Integer) AS Integer`) is
/// rejected, where a `Named("FUNC(…)")` blob would match coarsely and let it
/// through.
pub(crate) fn handler_type() -> ParameterType {
    ParameterType::Func(
        vec![ParameterType::named(REQUEST_TYPE)],
        Box::new(ParameterType::named(RESPONSE_TYPE)),
        false,
    )
}

/// A required `http` member parameter.
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

/// A trailing argument default-padded during IR lowering with `(ty, expr)` — the
/// registry `Fill` produces the exact `(type_name, expr)` pair the legacy
/// `default_argument_padding` injected (a `Map` default lowers to an empty map
/// literal, a scalar default to its const).
pub(crate) fn fill(
    name: &'static str,
    desc: &'static str,
    ty: ParameterType,
    expr: &'static str,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ty.clone(),
        default: DefaultValue::Fill {
            type_name: ty,
            expr,
        },
    }
}

/// The header map type (`Map OF String TO String`).
pub(crate) fn header_map() -> ParameterType {
    ParameterType::map_of(ParameterType::String, ParameterType::String)
}

const MODULE_INTRO: &str =
    r#"A blocking and non-blocking HTTP/1.1 client and a small HTTP/1.1 server"#;
const MODULE_DESC: &str = r#"The `http` package is a blocking and non-blocking HTTP/1.1 client and a small
HTTP/1.1 server, built entirely on the native `tcp`, `tls`, and `net` packages.
`http::read` and `http::write` perform a complete blocking request/response
exchange and return a `http::Response`; the five-call non-blocking client
(`startRead`/`ready`/`pump`/`done`/`finish`) drives an exchange without blocking
the calling thread. On the server side, `http::server`/`http::serverSSL` bind a
listener, `http::route` and `http::handleRequest` dispatch requests to handlers,
and the response constructors (`ok`/`status`/`json`/`withHeader`/`respondFile`/
`respondPath`) build a `http::Response`. `IMPORT http` needs no manifest dependency."#;

/// Register the `http` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("http", MODULE_INTRO, MODULE_DESC);

    // The companion source drives the wire protocol over `net`/`tls`, parses with
    // `strings`/`collections`, serves files with `fs`, and raises `errorCode` errors.
    pkg.add_imports(vec![
        // plan-110-E: `net` stays for the URL/query surface (`net::Url`,
        // `net::toUrl`, `net::percentDecode`, `net::parseQuery`), which did not
        // move; the transport moved to `tcp`.
        "net",
        "tcp",
        "tls",
        "fs",
        "strings",
        "collections",
        "errorCode",
        // bug-507: the server's whole-request deadline reads the monotonic clock.
        "datetime",
    ]);

    // The value types, registry-modeled (`get_mfb` renders them into the injected
    // source; the DOC blocks round-trip through `description`).
    pkg.add_record(RegistryRecord {
        name: RESPONSE_TYPE,
        export: true,
        description: "The client response from an HTTP request. A plain, copyable value record. `headers` is a standard map with field names lowercased (HTTP field names are case-insensitive), so a program reads a header with the ordinary collections accessors, e.g. `collections::getOr(resp.headers, \"content-type\", \"\")`. Duplicate fields collapse last-wins.",
        props: vec![
            RecordProp {
                name: "status",
                ty: ParameterType::Integer,
                description: "The HTTP status code, e.g. 200 or 404.",
            },
            RecordProp {
                name: "reason",
                ty: ParameterType::String,
                description: "The reason phrase, e.g. `\"OK\"`; `\"\"` if omitted.",
            },
            RecordProp {
                name: "httpVersion",
                ty: ParameterType::String,
                description: "The HTTP version from the status line, `\"1.0\"` or `\"1.1\"`.",
            },
            RecordProp {
                name: "headers",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::String),
                description: "Response headers, keyed by lowercased field name.",
            },
            RecordProp {
                name: "body",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "The raw body bytes (decode text via `toString`).",
            },
            RecordProp {
                name: "ok",
                ty: ParameterType::Boolean,
                description: "TRUE iff `status` is in 200..299.",
            },
        ],
    });
    // plan-76-D non-blocking client state (plan-74 union STATE, relocated to a
    // record slot free in every backend by plan-80).
    pkg.add_record(RegistryRecord {
        name: "PendingState",
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "sentAll",
                ty: ParameterType::Boolean,
                description: "Request fully written (reserved for a future write-pump).",
            },
            RecordProp {
                name: "closed",
                ty: ParameterType::Boolean,
                description: "Peer EOF observed (the Connection: close terminator).",
            },
            RecordProp {
                name: "raw",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "Accumulated response bytes.",
            },
            RecordProp {
                name: "err",
                ty: ParameterType::Integer,
                description: "0 = ok; else a captured transport failure code.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: REQUEST_TYPE,
        export: true,
        description: "A server-side request bound to a matched route. A flat, copyable value record. Header, query, and param maps use last-wins on duplicate keys.",
        props: vec![
            RecordProp {
                name: "method",
                ty: ParameterType::String,
                description: "The request method, uppercased (e.g. `\"GET\"`, `\"POST\"`).",
            },
            RecordProp {
                name: "path",
                ty: ParameterType::String,
                description: "The request path with the query stripped and percent-decoded.",
            },
            RecordProp {
                name: "rawPath",
                ty: ParameterType::String,
                description: "The request-target exactly as received.",
            },
            RecordProp {
                name: "headers",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::String),
                description: "Request headers, keyed by lowercased field name.",
            },
            RecordProp {
                name: "query",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::String),
                description: "Parsed query parameters from `?a=1&b=2`, decoded.",
            },
            RecordProp {
                name: "params",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::String),
                description: "Route captures (`:id`, `:x?`, `*`), keyed by capture name.",
            },
            RecordProp {
                name: "parts",
                ty: ParameterType::map_of(
                    ParameterType::String,
                    ParameterType::named("RequestPart"),
                ),
                description: "Multipart body parts, keyed by part name.",
            },
            RecordProp {
                name: "body",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "The raw request body bytes.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "RequestPart",
        export: true,
        description: "A single part of a multipart request body.",
        props: vec![
            RecordProp {
                name: "filename",
                ty: ParameterType::String,
                description: "The filename from Content-Disposition; `\"\"` for a plain field.",
            },
            RecordProp {
                name: "contentType",
                ty: ParameterType::String,
                description: "The part's Content-Type; `\"\"` if absent.",
            },
            RecordProp {
                name: "body",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "The part's raw bytes.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: ROUTE_TYPE,
        export: true,
        description:
            "A server route: a URL pattern paired with the handler that serves a matching request.",
        props: vec![
            RecordProp {
                name: "pattern",
                ty: ParameterType::String,
                description: "The route pattern, with captures like `:id`, `:x?`, and `*`.",
            },
            RecordProp {
                name: "handler",
                ty: ParameterType::func(
                    vec![ParameterType::named(REQUEST_TYPE)],
                    ParameterType::named(RESPONSE_TYPE),
                ),
                description:
                    "The function invoked with the bound `http::Request`, returning a `http::Response`.",
            },
        ],
    });
    // One non-blocking read's outcome, marshalled out of a top-level per-transport
    // helper. A `<call> TRAP` bound inside a `MATCH CASE` mis-types its temp as
    // `Unknown` at native codegen (plan-76-D Correction D3), so `pump`'s MATCH only
    // CALLS these helpers — the read + TRAP live at function top level.
    pkg.add_record(RegistryRecord {
        name: "__http_PumpRead",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "bytes",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "",
            },
            RecordProp {
                name: "closed",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "err",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });
    // Internal route-match result: the shared core cannot mutate a record field,
    // so matching produces the bound params here and `dispatch` rebuilds the
    // Request with `WITH`.
    pkg.add_record(RegistryRecord {
        name: "__http_RouteMatch",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "matched",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "params",
                ty: ParameterType::map_of(ParameterType::String, ParameterType::String),
                description: "",
            },
        ],
    });
    // bug-507: the server read loop's incremental framing state, threaded through
    // `__http_frameAdvance` after every read so the head scan and the chunk walk
    // resume where they stopped (OS-56). `status` is the 4xx to answer (0 = none).
    pkg.add_record(RegistryRecord {
        name: "__http_FrameState",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "status",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "complete",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "scanFrom",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "headEnd",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "framing",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "cursor",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });
    // bug-507: one connection's read outcome — the bytes and the 4xx to answer
    // (0 = a complete request to parse) — marshalled out of the per-transport
    // read loop so `handleRequest` can TRAP the whole read (OS-51).
    pkg.add_record(RegistryRecord {
        name: "__http_ReadResult",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "raw",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "",
            },
            RecordProp {
                name: "status",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });
    // plan-76-D: the non-blocking client's transport. `Stream` is a RESOURCE union
    // over the two transports; plan-97/bug-441 made built-in resources
    // package-qualified end to end, so the variants carry their qualified
    // identities (`tcp::Socket`, `tls::Socket`) and the close-wiring keys on
    // that same identity. It carries a `PendingState` (union STATE). A program
    // drives an exchange without blocking its thread:
    //   RES s AS http::Stream STATE PendingState = http::startRead(url, {}, "GET")
    //   WHILE http::done(s) = FALSE
    //     IF http::ready(s) THEN http::pump(s)   ' interleave the caller's own work
    //   END WHILE
    //   LET resp AS Response = http::finish(s)
    pkg.add_union(RegistryUnion {
        name: "Stream",
        export: true,
        variants: vec![
            UnionVariant {
                name: "tcp::Socket",
                description: "A plain-TCP exchange's transport.",
            },
            UnionVariant {
                name: "tls::Socket",
                description: "An HTTPS exchange's transport.",
            },
        ],
    });

    // The private `__http_*` helpers (protocol string work, framing, routing),
    // one `helper_*.rs` per FUNC/SUB (`add_helper` — private-only), registered
    // in the old companion order; the `__HTTP_*` limit globals render at their
    // old positions around them.
    helper_limits::register_response_limits(&mut pkg);
    helper_index_of::register(&mut pkg);
    helper_slice::register(&mut pkg);
    helper_default_port::register(&mut pkg);
    helper_normalize_method::register(&mut pkg);
    helper_host_header::register(&mut pkg);
    helper_request_target::register(&mut pkg);
    helper_header_value::register(&mut pkg);
    helper_is_extra_header::register(&mut pkg);
    helper_has_control_bytes::register(&mut pkg);
    helper_has_field_control_bytes::register(&mut pkg);
    helper_build_request::register(&mut pkg);
    helper_hex_to_int::register(&mut pkg);
    helper_dec_to_int::register(&mut pkg);
    helper_parse_status_line::register(&mut pkg);
    helper_parse_response::register(&mut pkg);
    helper_decode_body::register(&mut pkg);
    helper_read_net::register(&mut pkg);
    helper_read_tls::register(&mut pkg);
    helper_start_exchange::register(&mut pkg);
    helper_wait_readable::register(&mut pkg);
    helper_limits::register_request_limit(&mut pkg);
    helper_bytes_match_at::register(&mut pkg);
    helper_index_of_bytes::register(&mut pkg);
    helper_byte_slice::register(&mut pkg);
    helper_last_index_of::register(&mut pkg);
    helper_bytes_to_text::register(&mut pkg);
    helper_header_map_from_head::register(&mut pkg);
    helper_request_header_map::register(&mut pkg);
    helper_framing_length::register(&mut pkg);
    helper_request_framing::register(&mut pkg);
    helper_frame_complete::register(&mut pkg);
    helper_chunked_scan::register(&mut pkg);
    helper_chunked_complete::register(&mut pkg);
    helper_frame_start::register(&mut pkg);
    helper_frame_advance::register(&mut pkg);
    helper_read_request_net::register(&mut pkg);
    helper_read_request_tls::register(&mut pkg);
    helper_linger_net::register(&mut pkg);
    helper_linger_tls::register(&mut pkg);
    helper_dechunk_bytes::register(&mut pkg);
    helper_multipart_boundary::register(&mut pkg);
    helper_disposition_param::register(&mut pkg);
    helper_part_header::register(&mut pkg);
    helper_add_part::register(&mut pkg);
    helper_parse_multipart::register(&mut pkg);
    helper_empty_request::register(&mut pkg);
    helper_parse_request::register(&mut pkg);
    helper_normalize_path::register(&mut pkg);
    helper_segments::register(&mut pkg);
    helper_validate_pattern::register(&mut pkg);
    helper_match_path::register(&mut pkg);
    helper_invoke_handler::register(&mut pkg);
    helper_dispatch::register(&mut pkg);
    helper_check_response::register(&mut pkg);
    helper_build_response::register(&mut pkg);
    helper_response_with::register(&mut pkg);
    helper_reason_phrase::register(&mut pkg);
    helper_serialize_head::register(&mut pkg);
    helper_ext_content_type::register(&mut pkg);

    func_read::register(&mut pkg);
    func_write::register(&mut pkg);
    func_start_read::register(&mut pkg);
    func_ready::register(&mut pkg);
    func_pump::register(&mut pkg);
    func_done::register(&mut pkg);
    func_finish::register(&mut pkg);
    func_server::register(&mut pkg);
    func_server_ssl::register(&mut pkg);
    func_handle_request::register(&mut pkg);
    func_route::register(&mut pkg);
    func_response_default::register(&mut pkg);
    func_ok::register(&mut pkg);
    func_status::register(&mut pkg);
    func_json::register(&mut pkg);
    func_with_header::register(&mut pkg);
    func_bytes::register(&mut pkg);
    func_respond_file::register(&mut pkg);
    func_respond_path::register(&mut pkg);

    r.add_package(pkg);
}

/// The synthetic source path/doc labels — kept byte-identical to the legacy
/// `package_source_glue!("http", "<builtin-http>", "builtins/http.mfb", …)`.
const SOURCE_LABEL: &str = "<builtin-http>";
const SOURCE_DOC: &str = "builtins/http.mfb";

/// Inject the `http` source (the registry `get_mfb` assembly) as a dedicated late pass
/// (mirroring `net`/`encoding`): `http` is skipped by the generic single-pass
/// `registry::augment_project` so its `IMPORT net`/`IMPORT tls` transitive companions
/// are injected by their own passes, which scan the accumulated AST after this one.
pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    crate::codegen::registry::inject_late_pass(ast, "http", SOURCE_LABEL, SOURCE_DOC)
}

/// The same injection onto the elaborated project the former source checker consumes
/// (plan-106-D).
#[cfg(test)] // the HIR-domain chain serves the in-process tests only (plan-107-D)
pub(crate) fn augmented_hir_project(
    hir: &crate::hir::HirProject,
) -> Result<crate::hir::HirProject, ()> {
    crate::codegen::registry::inject_late_pass_hir(hir, "http", SOURCE_LABEL, SOURCE_DOC)
}
