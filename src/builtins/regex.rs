use std::borrow::Cow;
use std::path::Path;

const MATCH: &str = "regex.match";
const FIND: &str = "regex.find";
const FIND_ALL: &str = "regex.findAll";
const REPLACE: &str = "regex.replace";
const INTERNAL_MATCH: &str = "__regex_match";
const INTERNAL_FIND: &str = "__regex_find";
const INTERNAL_FIND_ALL: &str = "__regex_findAll";
const INTERNAL_REPLACE: &str = "__regex_replace";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_regex_call(name: &str) -> bool {
    matches!(name, MATCH | FIND | FIND_ALL | REPLACE)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        MATCH => Some(&[&["value"], &["pattern"]]),
        FIND | FIND_ALL => Some(&[&["value"], &["pattern"], &["start"]]),
        REPLACE => Some(&[&["value"], &["pattern"], &["replacement"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        MATCH => Some("Boolean"),
        FIND => Some("Integer"),
        FIND_ALL => Some("List OF Integer"),
        REPLACE => Some("String"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        MATCH if exact(arg_types, &["String", "String"]) => Cow::Borrowed("Boolean"),
        FIND if exact(arg_types, &["String", "String"])
            || exact(arg_types, &["String", "String", "Integer"]) =>
        {
            Cow::Borrowed("Integer")
        }
        FIND_ALL
            if exact(arg_types, &["String", "String"])
                || exact(arg_types, &["String", "String", "Integer"]) =>
        {
            Cow::Borrowed("List OF Integer")
        }
        REPLACE if exact(arg_types, &["String", "String", "String"]) => Cow::Borrowed("String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        MATCH => Some("String, String"),
        FIND | FIND_ALL => Some("String, String[, Integer]"),
        REPLACE => Some("String, String, String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        MATCH => Some((2, 2)),
        FIND | FIND_ALL => Some((2, 3)),
        REPLACE => Some((3, 3)),
        _ => None,
    }
}

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    match name {
        MATCH => Some(INTERNAL_MATCH),
        FIND => Some(INTERNAL_FIND),
        FIND_ALL => Some(INTERNAL_FIND_ALL),
        REPLACE => Some(INTERNAL_REPLACE),
        _ => None,
    }
}

/// Default trailing arguments injected during IR lowering so the internal
/// `__regex_find`/`__regex_findAll` always receive `start`. Mirrors the
/// `tls.connect` default-padding pattern.
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const FIND_DEFAULTS: &[(&str, &str)] = &[("Integer", "0")];
    match name {
        // find/findAll(value, pattern, [start=0])
        FIND | FIND_ALL => &FIND_DEFAULTS[provided.saturating_sub(2).min(FIND_DEFAULTS.len())..],
        _ => &[],
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    // The engine and the generated Unicode general-category table
    // (`regex_unicode.mfb`, see `scripts/gen_regex_unicode.py`) are kept as
    // separate physical files so the table can be regenerated mechanically, but
    // they compile as one source file: MFBASIC `FUNC`s are file-local unless
    // exported, and `PACKAGE` visibility is not valid in an executable, so the
    // engine's calls to `__regex_genCat` must be intra-file.
    let combined = format!(
        "{}\n{}",
        include_str!("regex_package.mfb"),
        include_str!("regex_unicode.mfb"),
    );
    crate::ast::parse_source_internal(
        Path::new("<builtin-regex>"),
        "builtins/regex.mfb",
        &combined,
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "regex")
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
    fn is_call_and_reject() {
        for n in [MATCH, FIND, FIND_ALL, REPLACE] {
            assert!(is_regex_call(n), "{n}");
        }
        assert!(!is_regex_call("regex.nope"));
        assert!(!is_regex_call(INTERNAL_MATCH));
    }

    #[test]
    fn param_names_branches() {
        assert_eq!(
            call_param_names(MATCH),
            Some(&[&["value"][..], &["pattern"]][..])
        );
        assert_eq!(
            call_param_names(FIND),
            Some(&[&["value"][..], &["pattern"], &["start"]][..])
        );
        assert_eq!(call_param_names(FIND), call_param_names(FIND_ALL));
        assert_eq!(
            call_param_names(REPLACE),
            Some(&[&["value"][..], &["pattern"], &["replacement"]][..])
        );
        assert!(call_param_names("regex.nope").is_none());
    }

    #[test]
    fn return_type_name_branches() {
        assert_eq!(call_return_type_name(MATCH), Some("Boolean"));
        assert_eq!(call_return_type_name(FIND), Some("Integer"));
        assert_eq!(call_return_type_name(FIND_ALL), Some("List OF Integer"));
        assert_eq!(call_return_type_name(REPLACE), Some("String"));
        assert!(call_return_type_name("regex.nope").is_none());
    }

    #[test]
    fn resolve_branches() {
        assert_eq!(
            rt(MATCH, &["String", "String"]),
            Some("Boolean".to_string())
        );
        assert_eq!(rt(FIND, &["String", "String"]), Some("Integer".to_string()));
        assert_eq!(
            rt(FIND, &["String", "String", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(FIND_ALL, &["String", "String"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(FIND_ALL, &["String", "String", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(REPLACE, &["String", "String", "String"]),
            Some("String".to_string())
        );
    }

    #[test]
    fn resolve_wrong_arity_or_type_none() {
        assert_eq!(rt(MATCH, &["String"]), None);
        assert_eq!(rt(MATCH, &["String", "Integer"]), None);
        assert_eq!(rt(FIND, &["String"]), None);
        assert_eq!(rt(REPLACE, &["String", "String"]), None);
        assert_eq!(rt("regex.nope", &["String", "String"]), None);
    }

    #[test]
    fn expected_arguments_branches() {
        assert_eq!(expected_arguments(MATCH), Some("String, String"));
        assert_eq!(expected_arguments(FIND), Some("String, String[, Integer]"));
        assert_eq!(
            expected_arguments(FIND_ALL),
            Some("String, String[, Integer]")
        );
        assert_eq!(expected_arguments(REPLACE), Some("String, String, String"));
        assert!(expected_arguments("regex.nope").is_none());
    }

    #[test]
    fn arity_branches() {
        assert_eq!(arity(MATCH), Some((2, 2)));
        assert_eq!(arity(FIND), Some((2, 3)));
        assert_eq!(arity(FIND_ALL), Some((2, 3)));
        assert_eq!(arity(REPLACE), Some((3, 3)));
        assert!(arity("regex.nope").is_none());
    }

    #[test]
    fn implementation_name_branches() {
        assert_eq!(implementation_name(MATCH), Some(INTERNAL_MATCH));
        assert_eq!(implementation_name(FIND), Some(INTERNAL_FIND));
        assert_eq!(implementation_name(FIND_ALL), Some(INTERNAL_FIND_ALL));
        assert_eq!(implementation_name(REPLACE), Some(INTERNAL_REPLACE));
        assert!(implementation_name("regex.nope").is_none());
    }

    #[test]
    fn default_padding_branches() {
        assert_eq!(default_argument_padding(FIND, 2).len(), 1);
        assert_eq!(default_argument_padding(FIND, 3).len(), 0);
        assert_eq!(default_argument_padding(FIND_ALL, 2).len(), 1);
        assert_eq!(default_argument_padding(MATCH, 2), &[]);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT regex\nSUB main\nEND SUB\n");
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
