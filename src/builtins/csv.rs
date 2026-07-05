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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file =
            crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn recognizes_csv_calls() {
        assert!(is_csv_call(PARSE));
        assert!(is_csv_call(STRINGIFY));
        assert!(!is_csv_call("csv.other"));
    }

    #[test]
    fn param_names_cover_all_calls() {
        assert_eq!(call_param_names(PARSE), Some(&[&["value", "text"][..]][..]));
        assert_eq!(call_param_names(STRINGIFY), Some(&[&["value"][..]][..]));
        assert_eq!(call_param_names("csv.other"), None);
    }

    #[test]
    fn return_types_and_arity() {
        assert_eq!(call_return_type_name(PARSE), Some(GRID_TYPE));
        assert_eq!(call_return_type_name(STRINGIFY), Some("String"));
        assert_eq!(call_return_type_name("csv.other"), None);
        assert_eq!(arity(PARSE), Some((1, 1)));
        assert_eq!(arity(STRINGIFY), Some((1, 1)));
        assert_eq!(arity("csv.other"), None);
    }

    #[test]
    fn resolve_call_branches() {
        assert_eq!(
            resolve_call(PARSE, &strings(&["String"])).map(|r| r.return_type.into_owned()),
            Some(GRID_TYPE.to_string())
        );
        assert_eq!(
            resolve_call(STRINGIFY, &strings(&[GRID_TYPE])).map(|r| r.return_type.into_owned()),
            Some("String".to_string())
        );
        assert!(resolve_call(PARSE, &strings(&["Integer"])).is_none());
        assert!(resolve_call(STRINGIFY, &strings(&["String"])).is_none());
        assert!(resolve_call("csv.other", &strings(&["String"])).is_none());
    }

    #[test]
    fn expected_arguments_and_impl_names() {
        assert_eq!(expected_arguments(PARSE), Some("String"));
        assert_eq!(expected_arguments(STRINGIFY), Some(GRID_TYPE));
        assert_eq!(expected_arguments("csv.other"), None);
        assert_eq!(implementation_name(PARSE), Some(INTERNAL_PARSE));
        assert_eq!(implementation_name(STRINGIFY), Some(INTERNAL_STRINGIFY));
        assert_eq!(implementation_name("csv.other"), None);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT csv\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len());
    }
}
