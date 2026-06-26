//! Front-end definitions for the built-in `http` package (plan-03-http.md): a
//! blocking HTTP/1.1 client. Like `json`/`csv`, `http` is a source package — this
//! thin Rust shim plus the MFBASIC implementation in `http_package.mfb`, injected
//! at compile time. Every byte on the wire goes through the existing native
//! `net`/`tls` packages; `http` introduces no new intrinsics.

use std::borrow::Cow;
use std::path::Path;

const READ: &str = "http.read";
const WRITE: &str = "http.write";

const INTERNAL_READ: &str = "__http_read";
const INTERNAL_WRITE: &str = "__http_write";

// The response value record. A plain, copyable record whose `headers` field is a
// standard `Map OF String TO String`, read with the ordinary collections
// accessors; there is no dedicated header function.
pub(crate) const RESPONSE_TYPE: &str = "Response";

const URL_TYPE: &str = "Url";
const HEADER_MAP: &str = "Map OF String TO String";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == RESPONSE_TYPE
}

pub(crate) fn is_http_call(name: &str) -> bool {
    matches!(name, READ | WRITE)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        READ => Some(&[&["url"], &["headers"], &["method"]]),
        WRITE => Some(&[&["url"], &["body"], &["headers"], &["method"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        READ | WRITE => Some(RESPONSE_TYPE),
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
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        READ => Some("Url, Map OF String TO String, String"),
        WRITE => Some("Url, String, Map OF String TO String, String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        READ => Some((1, 3)),
        WRITE => Some((2, 4)),
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
    match name {
        READ => &READ_DEFAULTS[provided.saturating_sub(1).min(READ_DEFAULTS.len())..],
        WRITE => &WRITE_DEFAULTS[provided.saturating_sub(2).min(WRITE_DEFAULTS.len())..],
        _ => &[],
    }
}

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    match name {
        READ => Some(INTERNAL_READ),
        WRITE => Some(INTERNAL_WRITE),
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
