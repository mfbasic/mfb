//! Front-end definitions for the built-in `http` package (plan-03-http.md): a
//! blocking HTTP/1.1 client. Like `json`/`csv`, `http` is a source package — this
//! thin Rust shim plus the MFBASIC implementation in `http_package.mfb`, injected
//! at compile time. Every byte on the wire goes through the existing native
//! `net`/`tls` packages; `http` introduces no new intrinsics.

use std::borrow::Cow;
use std::path::Path;

const READ: &str = "http.read";
const WRITE: &str = "http.write";
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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        RESPONSE_TYPE | REQUEST_TYPE | REQUEST_PART_TYPE | ROUTE_TYPE
    )
}

pub(crate) fn is_http_call(name: &str) -> bool {
    matches!(
        name,
        READ | WRITE
            | SERVER
            | SERVER_SSL
            | HANDLE_REQUEST
            | ROUTE
            | RESPONSE_DEFAULT
            | OK
            | STATUS
            | JSON
            | WITH_HEADER
            | BYTES
            | RESPOND_FILE
            | RESPOND_PATH
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        READ => Some(&[&["url"], &["headers"], &["method"]]),
        WRITE => Some(&[&["url"], &["body"], &["headers"], &["method"]]),
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

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        READ | WRITE => Some(RESPONSE_TYPE),
        SERVER => Some(LISTENER_TYPE),
        SERVER_SSL => Some(TLS_LISTENER_TYPE),
        HANDLE_REQUEST => Some("Nothing"),
        ROUTE => Some(ROUTE_TYPE),
        RESPONSE_DEFAULT | OK | STATUS | JSON | WITH_HEADER | RESPOND_FILE | RESPOND_PATH => {
            Some(RESPONSE_TYPE)
        }
        BYTES => Some(BYTE_LIST),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
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

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        READ => Some((1, 3)),
        WRITE => Some((2, 4)),
        SERVER => Some((1, 3)),
        SERVER_SSL => Some((3, 5)),
        HANDLE_REQUEST | STATUS | ROUTE | RESPOND_PATH => Some((2, 2)),
        WITH_HEADER => Some((3, 3)),
        RESPONSE_DEFAULT => Some((0, 0)),
        OK | JSON | BYTES => Some((1, 1)),
        RESPOND_FILE => Some((1, 2)),
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
/// listener type, so its target is chosen from the first argument's type; every
/// other call maps 1:1.
pub(crate) fn implementation_name(name: &str, arg_types: &[String]) -> Option<&'static str> {
    match name {
        READ => Some(INTERNAL_READ),
        WRITE => Some(INTERNAL_WRITE),
        SERVER => Some(INTERNAL_SERVER),
        SERVER_SSL => Some(INTERNAL_SERVER_SSL),
        HANDLE_REQUEST => {
            if arg_types.first().map(String::as_str) == Some(TLS_LISTENER_TYPE) {
                Some(INTERNAL_HANDLE_REQUEST_SSL)
            } else {
                Some(INTERNAL_HANDLE_REQUEST)
            }
        }
        ROUTE => Some(INTERNAL_ROUTE),
        RESPONSE_DEFAULT => Some(INTERNAL_RESPONSE_DEFAULT),
        OK => Some(INTERNAL_OK),
        STATUS => Some(INTERNAL_STATUS),
        JSON => Some(INTERNAL_JSON),
        WITH_HEADER => Some(INTERNAL_WITH_HEADER),
        BYTES => Some(INTERNAL_BYTES),
        RESPOND_FILE => Some(INTERNAL_RESPOND_FILE),
        RESPOND_PATH => Some(INTERNAL_RESPOND_PATH),
        _ => None,
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-http>"),
        "builtins/http.mfb",
        include_str!("http_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "http")
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

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
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
    fn return_type_name_branches() {
        assert_eq!(call_return_type_name(READ), Some(RESPONSE_TYPE));
        assert_eq!(call_return_type_name(WRITE), Some(RESPONSE_TYPE));
        assert!(call_return_type_name("http.nope").is_none());
    }

    #[test]
    fn resolve_read_overloads() {
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
        assert_eq!(rt(READ, &[URL_TYPE, "String"]), None);
        assert_eq!(rt(READ, &[]), None);
    }

    #[test]
    fn resolve_write_overloads() {
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
        assert_eq!(rt(WRITE, &[URL_TYPE]), None);
        assert_eq!(rt(WRITE, &["String", "String"]), None);
        assert_eq!(rt("http.nope", &[URL_TYPE]), None);
    }

    #[test]
    fn resolve_server_surface() {
        // server(port, [host], [backlog]) -> net::Listener
        assert_eq!(rt(SERVER, &["Integer"]), Some(LISTENER_TYPE.to_string()));
        assert_eq!(
            rt(SERVER, &["Integer", "String", "Integer"]),
            Some(LISTENER_TYPE.to_string())
        );
        assert_eq!(rt(SERVER, &["String"]), None);
        // serverSSL(port, certPath, keyPath, [host], [backlog]) -> tls::TlsListener
        assert_eq!(
            rt(SERVER_SSL, &["Integer", "String", "String"]),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        assert_eq!(
            rt(
                SERVER_SSL,
                &["Integer", "String", "String", "String", "Integer"]
            ),
            Some(TLS_LISTENER_TYPE.to_string())
        );
        assert_eq!(rt(SERVER_SSL, &["Integer", "String"]), None);
        // handleRequest overloaded by listener type -> Nothing
        assert_eq!(
            rt(HANDLE_REQUEST, &[LISTENER_TYPE, ROUTE_LIST]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            rt(HANDLE_REQUEST, &[TLS_LISTENER_TYPE, ROUTE_LIST]),
            Some("Nothing".to_string())
        );
        assert_eq!(rt(HANDLE_REQUEST, &[LISTENER_TYPE]), None);
        // route(pattern, handler: FUNC(Request) AS Response) -> Route
        assert_eq!(
            rt(ROUTE, &["String", HANDLER_TYPE]),
            Some(ROUTE_TYPE.to_string())
        );
        assert_eq!(rt(ROUTE, &["String", "FUNC(Integer) AS Integer"]), None);
        // constructors and static helpers
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
        assert_eq!(rt(RESPOND_PATH, &["String", "String"]), None);
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
    fn arity_branches() {
        assert_eq!(arity(READ), Some((1, 3)));
        assert_eq!(arity(WRITE), Some((2, 4)));
        assert!(arity("http.nope").is_none());
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
}
