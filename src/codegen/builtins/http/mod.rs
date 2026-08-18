//! The built-in `http` package (a blocking + non-blocking HTTP/1.1 client and a
//! small HTTP server) on the clean-room registry.
//!
//! Like `csv`/`json`, `http` is a **source package**: every member is pure MFBASIC
//! protocol string work in [`package.mfb`], reached through a `Body::Rewrite` to its
//! internal `__http_*` body. Every byte on the wire goes through the native
//! `net`/`tls` packages; `http` introduces no new intrinsics, resources, or runtime
//! specs.
//!
//! The value types (`Response`, `Request`, `RequestPart`, `Route`, the non-blocking
//! `Stream` union + its `PendingState`) are authored — with their `DOC` blocks — in
//! `package.mfb` and registered here as source types. `handleRequest` is overloaded
//! by listener type (`net::Listener` vs `tls::TlsListener`); that is two
//! `Implementation`s, each rewriting to its own transport body, selected by the
//! generic overload resolution (the datetime/net idiom, no custom resolver).

use crate::codegen::registry::{DefaultValue, Parameter, Registry, RegistryPackage};
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

/// The value-type names authored in `package.mfb`. The `Stream STATE PendingState`
/// composite is the stateful-resource string a non-blocking `Stream` carries.
pub(crate) const RESPONSE_TYPE: &str = "Response";
pub(crate) const REQUEST_TYPE: &str = "Request";
pub(crate) const ROUTE_TYPE: &str = "Route";
pub(crate) const STREAM_STATE: &str = "Stream STATE PendingState";
pub(crate) const LISTENER_TYPE: &str = "net.Listener";
pub(crate) const TLS_LISTENER_TYPE: &str = "tls.TlsListener";
pub(crate) const FILE_TYPE: &str = "fs.File";
pub(crate) const HANDLER_TYPE: &str = "FUNC(Request) AS Response";

/// A required `http` member parameter (docs live in `src/docs/man/builtins/http`).
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

/// A trailing argument default-padded during IR lowering with `(ty, expr)` — the
/// registry `Fill` produces the exact `(type_name, expr)` pair the legacy
/// `default_argument_padding` injected (a `Map` default lowers to an empty map
/// literal, a scalar default to its const).
pub(crate) fn fill(name: &'static str, ty: ParameterType, expr: &'static str) -> Parameter {
    Parameter {
        name,
        desc: "",
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
HTTP/1.1 server, built entirely on the native `net` and `tls` packages.
`http::read` and `http::write` perform a complete blocking request/response
exchange and return a `Response`; the five-call non-blocking client
(`startRead`/`ready`/`pump`/`done`/`finish`) drives an exchange without blocking
the calling thread. On the server side, `http::server`/`http::serverSSL` bind a
listener, `http::route` and `http::handleRequest` dispatch requests to handlers,
and the response constructors (`ok`/`status`/`json`/`withHeader`/`respondFile`/
`respondPath`) build a `Response`. `IMPORT http` needs no manifest dependency."#;

/// Register the `http` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("http", MODULE_INTRO, MODULE_DESC);

    // The companion source drives the wire protocol over `net`/`tls`, parses with
    // `strings`/`collections`, serves files with `fs`, and raises `errorCode` errors.
    pkg.add_imports(vec![
        "net",
        "tls",
        "fs",
        "strings",
        "collections",
        "errorCode",
    ]);

    // The value types are authored (with their DOC blocks) in `package.mfb`; the
    // registry records the names so the generic type query recognizes them.
    pkg.add_source_types(&[
        RESPONSE_TYPE,
        REQUEST_TYPE,
        "RequestPart",
        ROUTE_TYPE,
        "Stream",
        "PendingState",
    ]);

    // The whole `http` protocol implementation: the value-type declarations, the
    // private `__http_*` helpers, and every member body (reached by `Body::Rewrite`).
    pkg.add_helper(crate::codegen::registry::RegistryHelper::always(
        "http_package",
        include_str!("package.mfb"),
    ));

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

/// Inject the `http` source companion (`package.mfb`) as a dedicated late pass
/// (mirroring `net`/`encoding`): `http` is skipped by the generic single-pass
/// `registry::augment_project` so its `IMPORT net`/`IMPORT tls` transitive companions
/// are injected by their own passes, which scan the accumulated AST after this one.
pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    let Some(pkg) = crate::codegen::registry::registry().resolve_package("http") else {
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
