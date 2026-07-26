//! Built-in `app::` package seam (plan-62-A).
//!
//! `app::` makes an `--app` program's *presentation mode* a first-class,
//! extensible concept: `getMode` / `setMode` read and write the current mode,
//! chosen from the `Mode` enum (`Console` / `None`) declared in
//! `app_package.mfb`. The two callables are lowered inline in native codegen
//! (plan-62-B), reading and writing the per-arena presentation-mode field —
//! exactly like `money::getRounding` / `money::setRounding`. This module owns the
//! syntaxcheck metadata (arity, parameter names, return types) and the
//! source-package plumbing that makes the enum visible.
//!
//! The package is importable only in `--app` builds: `IMPORT app` in a plain
//! console build is a CLI compile error (plan-62-A §3.3), so the name gate here
//! makes the import *legal* and the CLI rejects it when app mode is off.

use std::borrow::Cow;

const GET_MODE: &str = "app.getMode";
const SET_MODE: &str = "app.setMode";

/// The public presentation-mode enum defined in `app_package.mfb`, referenced
/// bare (`Mode`) like every other builtin type.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == "Mode"
}

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_app_call(name: &str) -> bool {
    matches!(name, GET_MODE | SET_MODE)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        GET_MODE => &[],
        SET_MODE => &[&["mode"]],
        _ => return None,
    };
    Some(params)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    let type_ = match name {
        GET_MODE => "Mode",
        SET_MODE => "Nothing",
        _ => return None,
    };
    Some(type_)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type: &str = match name {
        GET_MODE if arg_types.is_empty() => "Mode",
        SET_MODE if exact(arg_types, &["Mode"]) => "Nothing",
        _ => return None,
    };
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        GET_MODE => "()",
        SET_MODE => "Mode",
        _ => return None,
    };
    Some(text)
}

/// The machine-readable positional argument-type signature (bug-340 A1): the
/// concrete per-parameter types IR lowering hands to `call_argument_expected_type`.
/// `getMode` takes no arguments (nothing to type), so it returns `None` — the same
/// shape `money::argument_types` uses for `getRounding`.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        SET_MODE => Some(&["Mode"]),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let span = match name {
        GET_MODE => (0, 0),
        SET_MODE => (1, 1),
        _ => return None,
    };
    Some(span)
}

super::package_source_glue!(
    "app",
    "<builtin-app>",
    "builtins/app.mfb",
    include_str!("app_package.mfb")
);

use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_the_two_callables() {
        assert_eq!(resolve_call(GET_MODE, &[]).unwrap().return_type, "Mode");
        assert_eq!(
            resolve_call(SET_MODE, &strings(&["Mode"]))
                .unwrap()
                .return_type,
            "Nothing"
        );
    }

    #[test]
    fn rejects_wrong_arguments() {
        assert!(resolve_call(GET_MODE, &strings(&["Mode"])).is_none());
        assert!(resolve_call(SET_MODE, &[]).is_none());
        assert!(resolve_call(SET_MODE, &strings(&["Integer"])).is_none());
        assert!(resolve_call("app.nope", &[]).is_none());
    }

    #[test]
    fn arity_and_type_metadata_present() {
        assert_eq!(arity(GET_MODE), Some((0, 0)));
        assert_eq!(arity(SET_MODE), Some((1, 1)));
        assert!(is_builtin_type("Mode"));
        assert!(!is_builtin_type("Console"));
        assert!(is_app_call(GET_MODE));
        assert!(is_app_call(SET_MODE));
        assert!(!is_app_call("app.nope"));
    }

    #[test]
    fn argument_types_machine_table() {
        // bug-340 A1: the machine-readable positional signature IR lowering reads.
        assert_eq!(argument_types(SET_MODE), Some(&["Mode"][..]));
        // getMode takes no arguments -> nothing to type.
        assert_eq!(argument_types(GET_MODE), None);
        assert_eq!(argument_types("app.nope"), None);
    }

    #[test]
    fn expected_arguments_table() {
        assert_eq!(expected_arguments(GET_MODE), Some("()"));
        assert_eq!(expected_arguments(SET_MODE), Some("Mode"));
        assert!(expected_arguments("app.nope").is_none());
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let file = crate::ast::parse_source(
            std::path::Path::new("main.mfb"),
            "main.mfb",
            "IMPORT app\nSUB main\nEND SUB\n",
        )
        .expect("parse source");
        let ast = crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }
}
