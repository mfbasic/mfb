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
        assert_eq!(implementation_name(READ), Some(INTERNAL_READ));
        assert_eq!(implementation_name(WRITE), Some(INTERNAL_WRITE));
        assert!(implementation_name("http.nope").is_none());
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
