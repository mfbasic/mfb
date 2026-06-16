use std::borrow::Cow;
use std::path::Path;

const PARSE: &str = "json.parse";
const STRINGIFY: &str = "json.stringify";
const GET: &str = "json.get";
const GET_OR: &str = "json.getOr";
const INTERNAL_PARSE: &str = "__json_parse";
const INTERNAL_STRINGIFY: &str = "__json_stringify";
const INTERNAL_GET: &str = "__json_get";
const INTERNAL_GET_OR: &str = "__json_getOr";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "Json" | "JsonNull" | "JsonBool" | "JsonNum" | "JsonStr" | "JsonArr" | "JsonObj"
    )
}

pub(crate) fn is_json_call(name: &str) -> bool {
    matches!(name, PARSE | STRINGIFY | GET | GET_OR)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        PARSE | GET | GET_OR => Some("Json"),
        STRINGIFY => Some("String"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        PARSE if exact(arg_types, &["String"]) => Cow::Borrowed("Json"),
        STRINGIFY if arg_types.len() == 1 && is_json_value_type(&arg_types[0]) => {
            Cow::Borrowed("String")
        }
        GET if arg_types.len() == 2
            && is_json_value_type(&arg_types[0])
            && arg_types[1] == "List OF String" =>
        {
            Cow::Borrowed("Json")
        }
        GET_OR
            if arg_types.len() == 3
                && is_json_value_type(&arg_types[0])
                && arg_types[1] == "List OF String"
                && is_json_value_type(&arg_types[2]) =>
        {
            Cow::Borrowed("Json")
        }
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some("String"),
        STRINGIFY => Some("Json"),
        GET => Some("Json, List OF String"),
        GET_OR => Some("Json, List OF String, Json"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        PARSE | STRINGIFY => Some((1, 1)),
        GET => Some((2, 2)),
        GET_OR => Some((3, 3)),
        _ => None,
    }
}

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some(INTERNAL_PARSE),
        STRINGIFY => Some(INTERNAL_STRINGIFY),
        GET => Some(INTERNAL_GET),
        GET_OR => Some(INTERNAL_GET_OR),
        _ => None,
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source(
        Path::new("<builtin-json>"),
        "builtins/json.mfb",
        include_str!("json_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "json")
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

fn is_json_value_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "Json" | "JsonNull" | "JsonBool" | "JsonNum" | "JsonStr" | "JsonArr" | "JsonObj"
    )
}
