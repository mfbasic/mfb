//! Front-end definitions for the built-in `http` package (plan-03-http.md): a
//! blocking HTTP/1.1 client. Like `json`/`csv`, `http` is a source package — this
//! thin Rust shim plus the MFBASIC implementation in `http_package.mfb`, injected
//! at compile time. Every byte on the wire goes through the existing native
//! `net`/`tls` packages; `http` introduces no new intrinsics.

use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    BuiltinType, DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType, TypeKind,
};

const READ: &str = "http.read";
const WRITE: &str = "http.write";
// plan-76-D: the non-blocking client. Five functions drive an HTTP exchange
// without blocking the calling thread, over a `Stream` resource union carrying a
// plan-74 `PendingState`. `read`/`write` are re-implemented over them (Phase 4).
const START_READ: &str = "http.startRead";
const READY: &str = "http.ready";
const PUMP: &str = "http.pump";
const DONE: &str = "http.done";
const FINISH: &str = "http.finish";
// Server surface (plan-05 §F.5): lifecycle, routing, response constructors,
// static-file helpers. The transport is the existing native `net`/`tls`
// packages; every function below is source logic in `http_package.mfb`.
const SERVER: &str = "http.server";
const SERVER_SSL: &str = "http.serverSSL";
const HANDLE_REQUEST: &str = "http.handleRequest";
const ROUTE: &str = "http.route";
const RESPONSE_DEFAULT: &str = "http.responseDefault";
const OK: &str = "http.ok";
const STATUS: &str = "http.status";
const JSON: &str = "http.json";
const WITH_HEADER: &str = "http.withHeader";
const BYTES: &str = "http.bytes";
const RESPOND_FILE: &str = "http.respondFile";
const RESPOND_PATH: &str = "http.respondPath";

const INTERNAL_READ: &str = "__http_read";
const INTERNAL_WRITE: &str = "__http_write";
const INTERNAL_START_READ: &str = "__http_startRead";
const INTERNAL_READY: &str = "__http_ready";
const INTERNAL_PUMP: &str = "__http_pump";
const INTERNAL_DONE: &str = "__http_done";
const INTERNAL_FINISH: &str = "__http_finish";
const INTERNAL_SERVER: &str = "__http_server";
const INTERNAL_SERVER_SSL: &str = "__http_serverSSL";
// `handleRequest` is overloaded by listener type (§F.5.1): the two transport
// bodies cannot share one socket variable, so each has its own internal target,
// selected in `implementation_name` by the first argument's type.
const INTERNAL_HANDLE_REQUEST: &str = "__http_handleRequest";
const INTERNAL_HANDLE_REQUEST_SSL: &str = "__http_handleRequestSSL";
const INTERNAL_ROUTE: &str = "__http_route";
const INTERNAL_RESPONSE_DEFAULT: &str = "__http_responseDefault";
const INTERNAL_OK: &str = "__http_ok";
const INTERNAL_STATUS: &str = "__http_status";
const INTERNAL_JSON: &str = "__http_json";
const INTERNAL_WITH_HEADER: &str = "__http_withHeader";
const INTERNAL_BYTES: &str = "__http_bytes";
const INTERNAL_RESPOND_FILE: &str = "__http_respondFile";
const INTERNAL_RESPOND_PATH: &str = "__http_respondPath";

// The response value record. A plain, copyable record whose `headers` field is a
// standard `Map OF String TO String`, read with the ordinary collections
// accessors; there is no dedicated header function. The client parser and the
// server response constructors build the same `Response` (§F.2.3).
pub(crate) const RESPONSE_TYPE: &str = "Response";
// plan-76-D: the non-blocking client's resource union + its plan-74 STATE record.
// `STREAM_STATE` is the full stateful-resource type string (`{type} STATE {state}`,
// bare ids) — the return of `startRead` and the parameter of `ready`/`pump`/`done`/
// `finish`. `Stream` is a RESOURCE union over `net::Socket`/`net::TlsSocket`
// (declared in `http_package.mfb`); plan-80 relocated the STATE slot so it works
// over the TlsSocket variant.
const STREAM_TYPE: &str = "Stream";
const PENDING_STATE_TYPE: &str = "PendingState";
const STREAM_STATE: &str = "Stream STATE PendingState";
// The server value records (§F.2). All flat, copyable, no resource fields.
pub(crate) const REQUEST_TYPE: &str = "Request";
pub(crate) const REQUEST_PART_TYPE: &str = "RequestPart";
pub(crate) const ROUTE_TYPE: &str = "Route";

const URL_TYPE: &str = "Url";
const HEADER_MAP: &str = "Map OF String TO String";
// Listener types the server binds/accepts on: the plaintext `net::Listener` and
// the TLS `tls::Listener`. Named here (already normalized to bare ids) to match
// the `handleRequest` overloads and the `server`/`serverSSL` return types.
const LISTENER_TYPE: &str = "Listener";
const TLS_LISTENER_TYPE: &str = "TlsListener";
const FILE_TYPE: &str = "File";
const BYTE_LIST: &str = "List OF Byte";
const ROUTE_LIST: &str = "List OF Route";
// The handler type every route stores: `FUNC(http::Request) AS http::Response`,
// normalized to bare ids at parse time (§A.1) — this is the exact type string a
// handler function reference resolves to.
const HANDLER_TYPE: &str = "FUNC(Request) AS Response";

// plan-72-M: `HTTP` is the descriptor authority for this package. Every function
// has a fixed return regardless of which overload matches, so
// `call_return_type_name` and `arity` derive from the descriptor. Optional
// trailing arguments are `DefaultValue::Fill` carrying the same `(type, expr)`
// pairs the legacy `default_argument_padding` injected, so default padding is
// DATA-derivable (see Corrections in plan-72-M — the resolver does not own it).
// `resolve_call` (overloads with type-union first arguments — `handleRequest`
// accepts `Listener` OR `TlsListener`) and typed `implementation_name`
// (`handleRequest` selects `__http_handleRequest{,SSL}` by the first argument's
// type) are argument-dependent and live on `HttpResolver`; every other call maps
// 1:1 to a fixed internal (`Implementation::Rewrite`). The parameter *types*
// below are illustrative where an overload is type-union; `resolve_call` owns the
// real acceptance. `Response`/`Request`/`RequestPart`/`Route` are the source
// companion record types; the `.mfb` companion injects on import (`WhenImported`).
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn hfn(
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

const fn req(name: &'static str, ty: &'static str) -> Parameter {
    Parameter::required(name, ty)
}

// A trailing argument that is default-padded during IR lowering with `(ty, expr)`.
const fn fill(name: &'static str, ty: &'static str, expr: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr,
        },
    }
}

const fn req_alias(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: &'static str,
) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}

const P_READ: &[Parameter] = &[
    req("url", URL_TYPE),
    fill("headers", HEADER_MAP, "{}"),
    fill("method", "String", "GET"),
];
// startRead mirrors read's arg contract (url, headers = {}, method = "GET").
const P_START_READ: &[Parameter] = &[
    req("url", URL_TYPE),
    fill("headers", HEADER_MAP, "{}"),
    fill("method", "String", "GET"),
];
// ready/pump/done/finish each take the bound stream by reference (the caller
// still owns it and drops it). The parameter type for CALL-SITE matching is the
// BASE union `Stream` — a resource value presents its base type for arg checks;
// the `STATE PendingState` suffix is carried on the internal `.mfb` parameter and
// resolved by plan-74's verifier/codegen (the builtin `resolve_call` path does
// exact string matching and does not subsume the STATE suffix). `RES`-ness is
// inferred from `Stream` being a resource union.
const P_STREAM: &[Parameter] = &[req("stream", STREAM_TYPE)];
const P_WRITE: &[Parameter] = &[
    req("url", URL_TYPE),
    req("body", "String"),
    fill("headers", HEADER_MAP, "{}"),
    fill("method", "String", "POST"),
];
const P_SERVER: &[Parameter] = &[
    req("port", "Integer"),
    fill("host", "String", "0.0.0.0"),
    fill("backlog", "Integer", "128"),
];
const P_SERVER_SSL: &[Parameter] = &[
    req("port", "Integer"),
    req("certPath", "String"),
    req("keyPath", "String"),
    fill("host", "String", "0.0.0.0"),
    fill("backlog", "Integer", "128"),
];
const P_HANDLE_REQUEST: &[Parameter] = &[
    req_alias("listener", &["server"], LISTENER_TYPE),
    req("routes", ROUTE_LIST),
];
const P_ROUTE: &[Parameter] = &[req("pattern", "String"), req("handler", HANDLER_TYPE)];
const P_BODY: &[Parameter] = &[req("body", "String")];
const P_STATUS: &[Parameter] = &[req("code", "Integer"), req("body", "String")];
const P_WITH_HEADER: &[Parameter] = &[
    req_alias("resp", &["response"], RESPONSE_TYPE),
    req("name", "String"),
    req("value", "String"),
];
const P_TEXT: &[Parameter] = &[req("text", "String")];
const P_RESPOND_FILE: &[Parameter] = &[req("file", FILE_TYPE), fill("contentType", "String", "")];
const P_RESPOND_PATH: &[Parameter] = &[
    req_alias("req", &["request"], REQUEST_TYPE),
    req("root", "String"),
];

const HTTP_FUNCTIONS: &[BuiltinFunction] = &[
    hfn(
        READ,
        "read",
        &[ov(P_READ, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_READ),
    ),
    hfn(
        WRITE,
        "write",
        &[ov(P_WRITE, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_WRITE),
    ),
    // plan-76-D: the five non-blocking client entry points.
    hfn(
        START_READ,
        "startRead",
        &[ov(P_START_READ, STREAM_STATE)],
        Implementation::Rewrite(INTERNAL_START_READ),
    ),
    hfn(
        READY,
        "ready",
        &[ov(P_STREAM, "Boolean")],
        Implementation::Rewrite(INTERNAL_READY),
    ),
    hfn(
        PUMP,
        "pump",
        &[ov(P_STREAM, "Nothing")],
        Implementation::Rewrite(INTERNAL_PUMP),
    ),
    hfn(
        DONE,
        "done",
        &[ov(P_STREAM, "Boolean")],
        Implementation::Rewrite(INTERNAL_DONE),
    ),
    hfn(
        FINISH,
        "finish",
        &[ov(P_STREAM, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_FINISH),
    ),
    hfn(
        SERVER,
        "server",
        &[ov(P_SERVER, LISTENER_TYPE)],
        Implementation::Rewrite(INTERNAL_SERVER),
    ),
    hfn(
        SERVER_SSL,
        "serverSSL",
        &[ov(P_SERVER_SSL, TLS_LISTENER_TYPE)],
        Implementation::Rewrite(INTERNAL_SERVER_SSL),
    ),
    // Overloaded by listener type: the resolver picks the internal target.
    hfn(
        HANDLE_REQUEST,
        "handleRequest",
        &[ov(P_HANDLE_REQUEST, "Nothing")],
        Implementation::Custom,
    ),
    hfn(
        ROUTE,
        "route",
        &[ov(P_ROUTE, ROUTE_TYPE)],
        Implementation::Rewrite(INTERNAL_ROUTE),
    ),
    hfn(
        RESPONSE_DEFAULT,
        "responseDefault",
        &[ov(&[], RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_RESPONSE_DEFAULT),
    ),
    hfn(
        OK,
        "ok",
        &[ov(P_BODY, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_OK),
    ),
    hfn(
        STATUS,
        "status",
        &[ov(P_STATUS, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_STATUS),
    ),
    hfn(
        JSON,
        "json",
        &[ov(P_BODY, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_JSON),
    ),
    hfn(
        WITH_HEADER,
        "withHeader",
        &[ov(P_WITH_HEADER, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_WITH_HEADER),
    ),
    hfn(
        BYTES,
        "bytes",
        &[ov(P_TEXT, BYTE_LIST)],
        Implementation::Rewrite(INTERNAL_BYTES),
    ),
    hfn(
        RESPOND_FILE,
        "respondFile",
        &[ov(P_RESPOND_FILE, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_RESPOND_FILE),
    ),
    hfn(
        RESPOND_PATH,
        "respondPath",
        &[ov(P_RESPOND_PATH, RESPONSE_TYPE)],
        Implementation::Rewrite(INTERNAL_RESPOND_PATH),
    ),
];

const HTTP_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: RESPONSE_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: REQUEST_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: REQUEST_PART_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: ROUTE_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
    // plan-76-D: the non-blocking client's types (variants/fields live in the
    // `.mfb`). `Stream` is a resource union (registered Opaque, like `json::Json`);
    // `PendingState` is its plan-74 STATE record.
    BuiltinType {
        name: STREAM_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: PENDING_STATE_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
];

/// The internal `__http_handleRequest{,SSL}` target: `handleRequest` is overloaded
/// by listener type, so the first argument's type selects the transport body. The
/// production `implementation_name` wrapper and the resolver share this.
fn handle_request_target(arg_types: &[String]) -> &'static str {
    if arg_types.first().map(String::as_str) == Some(TLS_LISTENER_TYPE) {
        INTERNAL_HANDLE_REQUEST_SSL
    } else {
        INTERNAL_HANDLE_REQUEST
    }
}

/// Argument-dependent resolution for http: overload validation with type-union
/// first arguments (return type) and the `handleRequest` typed target selection.
struct HttpResolver;
impl BuiltinResolver for HttpResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }

    fn implementation_name(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        (name == HANDLE_REQUEST).then(|| handle_request_target(arg_types).to_string())
    }
}
static HTTP_RESOLVER: HttpResolver = HttpResolver;

pub(crate) static HTTP: BuiltinModule = BuiltinModule {
    name: "http",
    functions: HTTP_FUNCTIONS,
    types: HTTP_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&HTTP_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    HTTP.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn is_http_call(name: &str) -> bool {
    DefaultResolver::contains(&HTTP, name)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        READ => Some(&[&["url"], &["headers"], &["method"]]),
        WRITE => Some(&[&["url"], &["body"], &["headers"], &["method"]]),
        START_READ => Some(&[&["url"], &["headers"], &["method"]]),
        READY | PUMP | DONE | FINISH => Some(&[&["stream"]]),
        SERVER => Some(&[&["port"], &["host"], &["backlog"]]),
        SERVER_SSL => Some(&[
            &["port"],
            &["certPath"],
            &["keyPath"],
            &["host"],
            &["backlog"],
        ]),
        HANDLE_REQUEST => Some(&[&["listener", "server"], &["routes"]]),
        ROUTE => Some(&[&["pattern"], &["handler"]]),
        RESPONSE_DEFAULT => Some(&[]),
        OK | JSON => Some(&[&["body"]]),
        STATUS => Some(&[&["code"], &["body"]]),
        WITH_HEADER => Some(&[&["resp", "response"], &["name"], &["value"]]),
        BYTES => Some(&[&["text"]]),
        RESPOND_FILE => Some(&[&["file"], &["contentType"]]),
        RESPOND_PATH => Some(&[&["req", "request"], &["root"]]),
        _ => None,
    }
}

/// Whether `arg_types` is the single bound-stream argument that
/// `ready`/`pump`/`done`/`finish` take. The parameter is the base union `Stream`,
/// but a real argument is a `Stream` value carrying its `PendingState`
/// (`Stream STATE PendingState`) — the only STATE a `Stream` ever has. Matching
/// on the base name (STATE stripped) rather than an exact `Stream` string is what
/// lets these resolve at a call site where the stream is spelled with its STATE
/// (e.g. a `FOR EACH` element of a `List OF RES Stream STATE PendingState`, or a
/// TRAP-desugared `$trap_res`); an exact-`Stream` match resolved those to
/// `Unknown`, which then had no native storage class (bug-429).
fn stream_arg(arg_types: &[String]) -> bool {
    arg_types.len() == 1
        && crate::builtins::resource::base_resource_name(&arg_types[0]) == STREAM_TYPE
}

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. `handleRequest` accepts either listener type; the
/// server/client overloads validate their per-position argument types.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        READ if exact(arg_types, &[URL_TYPE])
            || exact(arg_types, &[URL_TYPE, HEADER_MAP])
            || exact(arg_types, &[URL_TYPE, HEADER_MAP, "String"]) =>
        {
            Cow::Borrowed(RESPONSE_TYPE)
        }
        WRITE
            if exact(arg_types, &[URL_TYPE, "String"])
                || exact(arg_types, &[URL_TYPE, "String", HEADER_MAP])
                || exact(arg_types, &[URL_TYPE, "String", HEADER_MAP, "String"]) =>
        {
            Cow::Borrowed(RESPONSE_TYPE)
        }
        // plan-76-D non-blocking client. startRead mirrors read's arg forms and
        // returns the stateful `Stream` union; ready/done -> Boolean, pump ->
        // Nothing, finish -> Response. Each of the latter four takes the bound
        // stream (`Stream STATE PendingState`).
        START_READ
            if exact(arg_types, &[URL_TYPE])
                || exact(arg_types, &[URL_TYPE, HEADER_MAP])
                || exact(arg_types, &[URL_TYPE, HEADER_MAP, "String"]) =>
        {
            Cow::Borrowed(STREAM_STATE)
        }
        READY if stream_arg(arg_types) => Cow::Borrowed("Boolean"),
        PUMP if stream_arg(arg_types) => Cow::Borrowed("Nothing"),
        DONE if stream_arg(arg_types) => Cow::Borrowed("Boolean"),
        FINISH if stream_arg(arg_types) => Cow::Borrowed(RESPONSE_TYPE),
        // server(port, host = "0.0.0.0", backlog = 128) -> net::Listener
        SERVER
            if exact(arg_types, &["Integer"])
                || exact(arg_types, &["Integer", "String"])
                || exact(arg_types, &["Integer", "String", "Integer"]) =>
        {
            Cow::Borrowed(LISTENER_TYPE)
        }
        // serverSSL(port, certPath, keyPath, host = "0.0.0.0", backlog = 128) -> tls::Listener
        SERVER_SSL
            if exact(arg_types, &["Integer", "String", "String"])
                || exact(arg_types, &["Integer", "String", "String", "String"])
                || exact(
                    arg_types,
                    &["Integer", "String", "String", "String", "Integer"],
                ) =>
        {
            Cow::Borrowed(TLS_LISTENER_TYPE)
        }
        // handleRequest is overloaded by listener type; both feed the shared core.
        HANDLE_REQUEST
            if exact(arg_types, &[LISTENER_TYPE, ROUTE_LIST])
                || exact(arg_types, &[TLS_LISTENER_TYPE, ROUTE_LIST]) =>
        {
            Cow::Borrowed("Nothing")
        }
        ROUTE if exact(arg_types, &["String", HANDLER_TYPE]) => Cow::Borrowed(ROUTE_TYPE),
        RESPONSE_DEFAULT if arg_types.is_empty() => Cow::Borrowed(RESPONSE_TYPE),
        OK | JSON if exact(arg_types, &["String"]) => Cow::Borrowed(RESPONSE_TYPE),
        STATUS if exact(arg_types, &["Integer", "String"]) => Cow::Borrowed(RESPONSE_TYPE),
        WITH_HEADER if exact(arg_types, &[RESPONSE_TYPE, "String", "String"]) => {
            Cow::Borrowed(RESPONSE_TYPE)
        }
        BYTES if exact(arg_types, &["String"]) => Cow::Borrowed(BYTE_LIST),
        RESPOND_FILE
            if exact(arg_types, &[FILE_TYPE]) || exact(arg_types, &[FILE_TYPE, "String"]) =>
        {
            Cow::Borrowed(RESPONSE_TYPE)
        }
        RESPOND_PATH if exact(arg_types, &[REQUEST_TYPE, "String"]) => Cow::Borrowed(RESPONSE_TYPE),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        READ => Some("Url, Map OF String TO String, String"),
        WRITE => Some("Url, String, Map OF String TO String, String"),
        START_READ => Some("Url, Map OF String TO String, String"),
        READY | PUMP | DONE | FINISH => Some("Stream STATE PendingState"),
        // Bracketed/`or` forms are informational only — they are skipped for
        // literal coercion (the lowerer treats them as non-concrete).
        SERVER => Some("Integer[, String[, Integer]]"),
        SERVER_SSL => Some("Integer, String, String[, String[, Integer]]"),
        HANDLE_REQUEST => Some("Listener or TlsListener, List OF Route"),
        ROUTE => Some("String, FUNC(Request) AS Response"),
        RESPONSE_DEFAULT => Some("no arguments"),
        OK | JSON => Some("String"),
        STATUS => Some("Integer, String"),
        WITH_HEADER => Some("Response, String, String"),
        BYTES => Some("String"),
        RESPOND_FILE => Some("File[, String]"),
        RESPOND_PATH => Some("Request, String"),
        _ => None,
    }
}

/// Default trailing arguments injected during IR lowering: the empty `headers`
/// map then the method literal. The `Map OF String TO String` entry is lowered
/// to an empty map literal (not a scalar const) by the IR padding loop.
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const READ_DEFAULTS: &[(&str, &str)] = &[(HEADER_MAP, "{}"), ("String", "GET")];
    const WRITE_DEFAULTS: &[(&str, &str)] = &[(HEADER_MAP, "{}"), ("String", "POST")];
    // server(port, [host="0.0.0.0"], [backlog=128])
    const SERVER_DEFAULTS: &[(&str, &str)] = &[("String", "0.0.0.0"), ("Integer", "128")];
    // serverSSL(port, certPath, keyPath, [host="0.0.0.0"], [backlog=128])
    const SERVER_SSL_DEFAULTS: &[(&str, &str)] = &[("String", "0.0.0.0"), ("Integer", "128")];
    // respondFile(file, [contentType=""])
    const RESPOND_FILE_DEFAULTS: &[(&str, &str)] = &[("String", "")];
    match name {
        READ => &READ_DEFAULTS[provided.saturating_sub(1).min(READ_DEFAULTS.len())..],
        START_READ => &READ_DEFAULTS[provided.saturating_sub(1).min(READ_DEFAULTS.len())..],
        WRITE => &WRITE_DEFAULTS[provided.saturating_sub(2).min(WRITE_DEFAULTS.len())..],
        SERVER => &SERVER_DEFAULTS[provided.saturating_sub(1).min(SERVER_DEFAULTS.len())..],
        SERVER_SSL => {
            &SERVER_SSL_DEFAULTS[provided.saturating_sub(3).min(SERVER_SSL_DEFAULTS.len())..]
        }
        RESPOND_FILE => {
            &RESPOND_FILE_DEFAULTS[provided.saturating_sub(1).min(RESPOND_FILE_DEFAULTS.len())..]
        }
        _ => &[],
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `respondFile` takes ownership of the `RES File` it serves (§F.5.5), closing
/// it by lexical drop; every other server call only uses the handle.
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!((name, index), (RESPOND_FILE, 0))
}

/// The internal source-companion target. `handleRequest` is overloaded by
/// listener type, so its target is chosen from the first argument's type (owned by
/// `HttpResolver` and the shared `handle_request_target`); every other call maps
/// 1:1 to its `Implementation::Rewrite` symbol in the descriptor.
pub(crate) fn implementation_name(name: &str, arg_types: &[String]) -> Option<&'static str> {
    match name {
        HANDLE_REQUEST => Some(handle_request_target(arg_types)),
        _ => DefaultResolver::implementation_name(&HTTP, name),
    }
}

super::package_source_glue!(
    "http",
    "<builtin-http>",
    "builtins/http.mfb",
    include_str!("http_package.mfb")
);

use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
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
    fn builtin_type_and_is_call() {
        assert!(is_builtin_type(RESPONSE_TYPE));
        assert!(!is_builtin_type("Url"));
        assert!(is_http_call(READ));
        assert!(is_http_call(WRITE));
        assert!(!is_http_call("http.nope"));
    }

    #[test]
    fn param_names_branches() {
        assert_eq!(
            call_param_names(READ),
            Some(&[&["url"][..], &["headers"], &["method"]][..])
        );
        assert_eq!(
            call_param_names(WRITE),
            Some(&[&["url"][..], &["body"], &["headers"], &["method"]][..])
        );
        assert!(call_param_names("http.nope").is_none());
    }

    #[test]
    fn server_types_and_consumes() {
        assert!(is_builtin_type(REQUEST_TYPE));
        assert!(is_builtin_type(REQUEST_PART_TYPE));
        assert!(is_builtin_type(ROUTE_TYPE));
        assert!(is_http_call(SERVER));
        assert!(is_http_call(HANDLE_REQUEST));
        assert!(is_http_call(RESPOND_PATH));
        // respondFile consumes its RES File; nothing else consumes.
        assert!(consumes_argument(RESPOND_FILE, 0));
        assert!(!consumes_argument(RESPOND_FILE, 1));
        assert!(!consumes_argument(HANDLE_REQUEST, 0));
        // default padding for the defaulted server calls.
        assert_eq!(default_argument_padding(SERVER, 1).len(), 2);
        assert_eq!(default_argument_padding(SERVER, 3).len(), 0);
        assert_eq!(default_argument_padding(SERVER_SSL, 3).len(), 2);
        assert_eq!(default_argument_padding(RESPOND_FILE, 1).len(), 1);
    }

    #[test]
    fn expected_arguments_branches() {
        assert_eq!(
            expected_arguments(READ),
            Some("Url, Map OF String TO String, String")
        );
        assert_eq!(
            expected_arguments(WRITE),
            Some("Url, String, Map OF String TO String, String")
        );
        assert!(expected_arguments("http.nope").is_none());
    }

    #[test]
    fn default_padding_branches() {
        // read(url, [headers={}], [method=GET])
        assert_eq!(default_argument_padding(READ, 1).len(), 2);
        assert_eq!(default_argument_padding(READ, 2).len(), 1);
        assert_eq!(default_argument_padding(READ, 3).len(), 0);
        // write(url, body, [headers={}], [method=POST])
        assert_eq!(default_argument_padding(WRITE, 2).len(), 2);
        assert_eq!(default_argument_padding(WRITE, 3).len(), 1);
        assert_eq!(default_argument_padding(WRITE, 4).len(), 0);
        assert_eq!(default_argument_padding("http.nope", 1), &[]);
    }

    #[test]
    fn implementation_name_branches() {
        assert_eq!(implementation_name(READ, &[]), Some(INTERNAL_READ));
        assert_eq!(implementation_name(WRITE, &[]), Some(INTERNAL_WRITE));
        assert_eq!(implementation_name(SERVER, &[]), Some(INTERNAL_SERVER));
        assert_eq!(
            implementation_name(SERVER_SSL, &[]),
            Some(INTERNAL_SERVER_SSL)
        );
        // handleRequest routes by first-argument listener type.
        assert_eq!(
            implementation_name(HANDLE_REQUEST, &strings(&[LISTENER_TYPE, ROUTE_LIST])),
            Some(INTERNAL_HANDLE_REQUEST)
        );
        assert_eq!(
            implementation_name(HANDLE_REQUEST, &strings(&[TLS_LISTENER_TYPE, ROUTE_LIST])),
            Some(INTERNAL_HANDLE_REQUEST_SSL)
        );
        assert_eq!(implementation_name(ROUTE, &[]), Some(INTERNAL_ROUTE));
        assert_eq!(
            implementation_name(RESPONSE_DEFAULT, &[]),
            Some(INTERNAL_RESPONSE_DEFAULT)
        );
        assert_eq!(implementation_name(OK, &[]), Some(INTERNAL_OK));
        assert_eq!(implementation_name(STATUS, &[]), Some(INTERNAL_STATUS));
        assert_eq!(implementation_name(JSON, &[]), Some(INTERNAL_JSON));
        assert_eq!(
            implementation_name(WITH_HEADER, &[]),
            Some(INTERNAL_WITH_HEADER)
        );
        assert_eq!(implementation_name(BYTES, &[]), Some(INTERNAL_BYTES));
        assert_eq!(
            implementation_name(RESPOND_FILE, &[]),
            Some(INTERNAL_RESPOND_FILE)
        );
        assert_eq!(
            implementation_name(RESPOND_PATH, &[]),
            Some(INTERNAL_RESPOND_PATH)
        );
        assert!(implementation_name("http.nope", &[]).is_none());
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT http\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len() + 1
        );
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

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        dispatch_resolve(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov`/`hfn`/`req`/`fill`/`req_alias` are const fns invoked only in const
        // context, so their bodies never run at runtime. Call them here to exercise
        // (and pin the shape of) each constructor.
        let overload = ov(P_READ, RESPONSE_TYPE);
        assert_eq!(overload.params.len(), 3);
        assert_eq!(overload.params[0].name, "url");
        assert_eq!(overload.return_type, ReturnType::Fixed(RESPONSE_TYPE));

        const OV: &[BuiltinOverload] = &[ov(P_BODY, RESPONSE_TYPE)];
        let func = hfn(OK, "ok", OV, Implementation::Rewrite(INTERNAL_OK));
        assert_eq!(func.name, OK);
        assert_eq!(func.doc_slug, "ok");
        assert_eq!(func.implementation, Implementation::Rewrite(INTERNAL_OK));
        assert_eq!(func.lowering, Lowering::Helper);
        assert_eq!(func.overloads.len(), 1);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);

        let r = req("url", URL_TYPE);
        assert_eq!(r.name, "url");
        assert_eq!(r.ty, ParameterType::Named(URL_TYPE));
        assert_eq!(r.default, DefaultValue::None);
        assert!(r.aliases.is_empty());

        let f = fill("headers", HEADER_MAP, "{}");
        assert_eq!(f.name, "headers");
        assert_eq!(f.ty, ParameterType::Named(HEADER_MAP));
        assert_eq!(
            f.default,
            DefaultValue::Fill {
                type_name: HEADER_MAP,
                expr: "{}"
            }
        );
        assert!(f.aliases.is_empty());

        const ALIASES: &[&str] = &["response"];
        let a = req_alias("resp", ALIASES, RESPONSE_TYPE);
        assert_eq!(a.name, "resp");
        assert_eq!(a.aliases, ALIASES);
        assert_eq!(a.ty, ParameterType::Named(RESPONSE_TYPE));
        assert_eq!(a.default, DefaultValue::None);
    }

    #[test]
    fn resolver_trait_dispatches() {
        // The HttpResolver trait methods are wired through the descriptor; call
        // them directly to cover `resolve_return_type` and `implementation_name`.
        assert_eq!(
            HTTP_RESOLVER.resolve_return_type(&HTTP, READ, &strings(&[URL_TYPE])),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            HTTP_RESOLVER.resolve_return_type(&HTTP, READ, &strings(&["String"])),
            None
        );
        // implementation_name is Some only for handleRequest, choosing the transport.
        assert_eq!(
            HTTP_RESOLVER.implementation_name(
                &HTTP,
                HANDLE_REQUEST,
                &strings(&[LISTENER_TYPE, ROUTE_LIST])
            ),
            Some(INTERNAL_HANDLE_REQUEST.to_string())
        );
        assert_eq!(
            HTTP_RESOLVER.implementation_name(
                &HTTP,
                HANDLE_REQUEST,
                &strings(&[TLS_LISTENER_TYPE, ROUTE_LIST])
            ),
            Some(INTERNAL_HANDLE_REQUEST_SSL.to_string())
        );
        assert_eq!(
            HTTP_RESOLVER.implementation_name(&HTTP, READ, &strings(&[URL_TYPE])),
            None
        );
    }

    #[test]
    fn dispatch_resolve_every_overload() {
        // read(url, [headers], [method]) -> Response
        assert_eq!(rt(READ, &[URL_TYPE]), Some(RESPONSE_TYPE.to_string()));
        assert_eq!(
            rt(READ, &[URL_TYPE, HEADER_MAP]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            rt(READ, &[URL_TYPE, HEADER_MAP, "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(rt(READ, &["String"]), None);
        // write(url, body, [headers], [method]) -> Response
        assert_eq!(
            rt(WRITE, &[URL_TYPE, "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            rt(WRITE, &[URL_TYPE, "String", HEADER_MAP]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            rt(WRITE, &[URL_TYPE, "String", HEADER_MAP, "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        // server(port, [host], [backlog]) -> Listener
        assert_eq!(rt(SERVER, &["Integer"]), Some(LISTENER_TYPE.to_string()));
        assert_eq!(
            rt(SERVER, &["Integer", "String"]),
            Some(LISTENER_TYPE.to_string())
        );
        assert_eq!(
            rt(SERVER, &["Integer", "String", "Integer"]),
            Some(LISTENER_TYPE.to_string())
        );
        // serverSSL(port, certPath, keyPath, [host], [backlog]) -> TlsListener
        assert_eq!(
            rt(SERVER_SSL, &["Integer", "String", "String"]),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        assert_eq!(
            rt(SERVER_SSL, &["Integer", "String", "String", "String"]),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        assert_eq!(
            rt(
                SERVER_SSL,
                &["Integer", "String", "String", "String", "Integer"]
            ),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        // handleRequest overloaded by listener type -> Nothing
        assert_eq!(
            rt(HANDLE_REQUEST, &[LISTENER_TYPE, ROUTE_LIST]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            rt(HANDLE_REQUEST, &[TLS_LISTENER_TYPE, ROUTE_LIST]),
            Some("Nothing".to_string())
        );
        // constructors / static helpers
        assert_eq!(
            rt(ROUTE, &["String", HANDLER_TYPE]),
            Some(ROUTE_TYPE.to_string())
        );
        assert_eq!(rt(RESPONSE_DEFAULT, &[]), Some(RESPONSE_TYPE.to_string()));
        assert_eq!(rt(OK, &["String"]), Some(RESPONSE_TYPE.to_string()));
        assert_eq!(rt(JSON, &["String"]), Some(RESPONSE_TYPE.to_string()));
        assert_eq!(
            rt(STATUS, &["Integer", "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            rt(WITH_HEADER, &[RESPONSE_TYPE, "String", "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(rt(BYTES, &["String"]), Some(BYTE_LIST.to_string()));
        assert_eq!(
            rt(RESPOND_FILE, &[FILE_TYPE]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            rt(RESPOND_FILE, &[FILE_TYPE, "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        assert_eq!(
            rt(RESPOND_PATH, &[REQUEST_TYPE, "String"]),
            Some(RESPONSE_TYPE.to_string())
        );
        // reject paths (final `_ => return None` arm)
        assert_eq!(rt("http.nope", &[]), None);
        assert_eq!(rt(ROUTE, &["String", "FUNC(Integer) AS Integer"]), None);
    }

    #[test]
    fn expected_arguments_server_surface() {
        assert_eq!(
            expected_arguments(SERVER),
            Some("Integer[, String[, Integer]]")
        );
        assert_eq!(
            expected_arguments(SERVER_SSL),
            Some("Integer, String, String[, String[, Integer]]")
        );
        assert_eq!(
            expected_arguments(HANDLE_REQUEST),
            Some("Listener or TlsListener, List OF Route")
        );
        assert_eq!(
            expected_arguments(ROUTE),
            Some("String, FUNC(Request) AS Response")
        );
        assert_eq!(expected_arguments(RESPONSE_DEFAULT), Some("no arguments"));
        assert_eq!(expected_arguments(OK), Some("String"));
        assert_eq!(expected_arguments(JSON), Some("String"));
        assert_eq!(expected_arguments(STATUS), Some("Integer, String"));
        assert_eq!(
            expected_arguments(WITH_HEADER),
            Some("Response, String, String")
        );
        assert_eq!(expected_arguments(BYTES), Some("String"));
        assert_eq!(expected_arguments(RESPOND_FILE), Some("File[, String]"));
        assert_eq!(expected_arguments(RESPOND_PATH), Some("Request, String"));
    }
}
