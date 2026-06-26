use std::borrow::Cow;
use std::path::Path;

const PARSE: &str = "csv.parse";
const STRINGIFY: &str = "csv.stringify";
const INTERNAL_PARSE: &str = "__csv_parse";
const INTERNAL_STRINGIFY: &str = "__csv_stringify";

const GRID_TYPE: &str = "List OF List OF String";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_csv_call(name: &str) -> bool {
    matches!(name, PARSE | STRINGIFY)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        PARSE => Some(&[&["value", "text"]]),
        STRINGIFY => Some(&[&["value"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some(GRID_TYPE),
        STRINGIFY => Some("String"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        PARSE if exact(arg_types, &["String"]) => Cow::Borrowed(GRID_TYPE),
        STRINGIFY if exact(arg_types, &[GRID_TYPE]) => Cow::Borrowed("String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some("String"),
        STRINGIFY => Some(GRID_TYPE),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        PARSE | STRINGIFY => Some((1, 1)),
        _ => None,
    }
}

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some(INTERNAL_PARSE),
        STRINGIFY => Some(INTERNAL_STRINGIFY),
        _ => None,
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-csv>"),
        "builtins/csv.mfb",
        include_str!("csv_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "csv")
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
