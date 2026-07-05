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

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        PARSE => Some(&[&["value", "text"]]),
        STRINGIFY => Some(&[&["value"]]),
        GET => Some(&[&["value"], &["path", "key"]]),
        GET_OR => Some(&[
            &["value"],
            &["path", "key"],
            &["default", "defaultValue", "fallback"],
        ]),
        _ => None,
    }
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
    crate::ast::parse_source_internal(
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

    fn returns(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn is_builtin_type_covers_json_family() {
        for name in [
            "Json", "JsonNull", "JsonBool", "JsonNum", "JsonStr", "JsonArr", "JsonObj",
        ] {
            assert!(is_builtin_type(name));
            assert!(is_json_value_type(name));
        }
        assert!(!is_builtin_type("String"));
        assert!(!is_json_value_type("Integer"));
    }

    #[test]
    fn recognizes_json_calls() {
        assert!(is_json_call(PARSE));
        assert!(is_json_call(STRINGIFY));
        assert!(is_json_call(GET));
        assert!(is_json_call(GET_OR));
        assert!(!is_json_call("json.other"));
    }

    #[test]
    fn param_names_cover_all_calls() {
        assert_eq!(call_param_names(PARSE), Some(&[&["value", "text"][..]][..]));
        assert_eq!(call_param_names(STRINGIFY), Some(&[&["value"][..]][..]));
        assert_eq!(
            call_param_names(GET),
            Some(&[&["value"][..], &["path", "key"][..]][..])
        );
        assert!(call_param_names(GET_OR).is_some());
        assert_eq!(call_param_names("json.other"), None);
    }

    #[test]
    fn return_types_and_arity() {
        assert_eq!(call_return_type_name(PARSE), Some("Json"));
        assert_eq!(call_return_type_name(GET), Some("Json"));
        assert_eq!(call_return_type_name(GET_OR), Some("Json"));
        assert_eq!(call_return_type_name(STRINGIFY), Some("String"));
        assert_eq!(call_return_type_name("json.other"), None);
        assert_eq!(arity(PARSE), Some((1, 1)));
        assert_eq!(arity(STRINGIFY), Some((1, 1)));
        assert_eq!(arity(GET), Some((2, 2)));
        assert_eq!(arity(GET_OR), Some((3, 3)));
        assert_eq!(arity("json.other"), None);
    }

    #[test]
    fn resolve_call_accepts_valid_signatures() {
        assert_eq!(returns(PARSE, &["String"]), Some("Json".to_string()));
        assert_eq!(returns(STRINGIFY, &["Json"]), Some("String".to_string()));
        assert_eq!(returns(STRINGIFY, &["JsonObj"]), Some("String".to_string()));
        assert_eq!(
            returns(GET, &["Json", "List OF String"]),
            Some("Json".to_string())
        );
        assert_eq!(
            returns(GET_OR, &["Json", "List OF String", "JsonStr"]),
            Some("Json".to_string())
        );
    }

    #[test]
    fn resolve_call_rejects_bad_signatures() {
        assert!(returns(PARSE, &["Integer"]).is_none());
        assert!(returns(STRINGIFY, &["String"]).is_none());
        assert!(returns(GET, &["Json", "String"]).is_none());
        assert!(returns(GET, &["String", "List OF String"]).is_none());
        assert!(returns(GET_OR, &["Json", "List OF String", "String"]).is_none());
        assert!(returns(GET_OR, &["Json", "Integer", "Json"]).is_none());
        assert!(returns("json.other", &["String"]).is_none());
    }

    #[test]
    fn expected_arguments_and_impl_names() {
        assert_eq!(expected_arguments(PARSE), Some("String"));
        assert_eq!(expected_arguments(STRINGIFY), Some("Json"));
        assert_eq!(expected_arguments(GET), Some("Json, List OF String"));
        assert_eq!(
            expected_arguments(GET_OR),
            Some("Json, List OF String, Json")
        );
        assert_eq!(expected_arguments("json.other"), None);
        assert_eq!(implementation_name(PARSE), Some(INTERNAL_PARSE));
        assert_eq!(implementation_name(STRINGIFY), Some(INTERNAL_STRINGIFY));
        assert_eq!(implementation_name(GET), Some(INTERNAL_GET));
        assert_eq!(implementation_name(GET_OR), Some(INTERNAL_GET_OR));
        assert_eq!(implementation_name("json.other"), None);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT json\nSUB main\nEND SUB\n");
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
