//! Built-in `money::` package seam (plan-29-D).
//!
//! `money::` controls how Money *arithmetic* settles the half case. The
//! `Rounding` enum (`Commercial` / `Banker`) is declared in
//! `money_package.mfb`; the three callables — `setRounding`, `getRounding`, and
//! `round` — are lowered inline in native codegen (`builder_money`), reading and
//! writing the per-arena rounding-mode field. This module owns the syntaxcheck
//! metadata (arity, parameter names, return types) and the source-package
//! plumbing that makes the enum visible.

use std::borrow::Cow;
use std::path::Path;

const SET_ROUNDING: &str = "money.setRounding";
const GET_ROUNDING: &str = "money.getRounding";
const ROUND: &str = "money.round";

/// The public rounding-mode enum defined in `money_package.mfb`, referenced bare
/// (`Rounding`) like every other builtin type.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == "Rounding"
}

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_money_call(name: &str) -> bool {
    matches!(name, SET_ROUNDING | GET_ROUNDING | ROUND)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        SET_ROUNDING => &[&["mode"]],
        GET_ROUNDING => &[],
        ROUND => &[&["value"], &["decimals"]],
        _ => return None,
    };
    Some(params)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    let type_ = match name {
        SET_ROUNDING => "Nothing",
        GET_ROUNDING => "Rounding",
        ROUND => "Money",
        _ => return None,
    };
    Some(type_)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type: &str = match name {
        SET_ROUNDING if exact(arg_types, &["Rounding"]) => "Nothing",
        GET_ROUNDING if arg_types.is_empty() => "Rounding",
        ROUND if exact(arg_types, &["Money", "Integer"]) => "Money",
        _ => return None,
    };
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        SET_ROUNDING => "Rounding",
        GET_ROUNDING => "()",
        ROUND => "Money, Integer",
        _ => return None,
    };
    Some(text)
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let span = match name {
        SET_ROUNDING => (1, 1),
        GET_ROUNDING => (0, 0),
        ROUND => (2, 2),
        _ => return None,
    };
    Some(span)
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-money>"),
        "builtins/money.mfb",
        include_str!("money_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "money")
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

    #[test]
    fn resolves_the_three_callables() {
        assert_eq!(
            resolve_call(SET_ROUNDING, &strings(&["Rounding"]))
                .unwrap()
                .return_type,
            "Nothing"
        );
        assert_eq!(
            resolve_call(GET_ROUNDING, &[]).unwrap().return_type,
            "Rounding"
        );
        assert_eq!(
            resolve_call(ROUND, &strings(&["Money", "Integer"]))
                .unwrap()
                .return_type,
            "Money"
        );
    }

    #[test]
    fn rejects_wrong_arguments() {
        assert!(resolve_call(SET_ROUNDING, &strings(&["Integer"])).is_none());
        assert!(resolve_call(GET_ROUNDING, &strings(&["Integer"])).is_none());
        assert!(resolve_call(ROUND, &strings(&["Money"])).is_none());
        assert!(resolve_call(ROUND, &strings(&["Integer", "Integer"])).is_none());
    }

    #[test]
    fn arity_and_type_metadata_present() {
        assert_eq!(arity(SET_ROUNDING), Some((1, 1)));
        assert_eq!(arity(GET_ROUNDING), Some((0, 0)));
        assert_eq!(arity(ROUND), Some((2, 2)));
        assert!(is_builtin_type("Rounding"));
        assert!(!is_builtin_type("Money"));
        assert!(is_money_call(ROUND));
        assert!(!is_money_call("money.nope"));
    }
}
