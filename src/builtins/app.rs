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

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, BuiltinType,
    DefaultResolver, Implementation, InjectionRule, Lowering, Parameter, ReturnType, TypeKind,
};

const GET_MODE: &str = "app.getMode";
const SET_MODE: &str = "app.setMode";

// plan-72-B: `APP` is the descriptor authority for this package. `getMode` and
// `setMode` both lower inline in native codegen (plan-62-B), so neither has an
// implementation-name rewrite. `Mode` is the presentation-mode enum declared in
// `app_package.mfb`, exposed as a builtin type name; its `.mfb` companion is
// injected whenever `app` is imported.
const APP_FUNCTIONS: &[BuiltinFunction] = &[
    BuiltinFunction {
        name: GET_MODE,
        doc_slug: "getMode",
        overloads: &[BuiltinOverload {
            params: &[],
            return_type: ReturnType::Fixed("Mode"),
        }],
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
    BuiltinFunction {
        name: SET_MODE,
        doc_slug: "setMode",
        overloads: &[BuiltinOverload {
            params: &[Parameter::required("mode", "Mode")],
            return_type: ReturnType::Fixed("Nothing"),
        }],
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
];

const APP_TYPES: &[BuiltinType] = &[BuiltinType {
    name: "Mode",
    kind: TypeKind::Enum,
    fields: &[],
}];

pub(crate) static APP: BuiltinModule = BuiltinModule {
    name: "app",
    functions: APP_FUNCTIONS,
    types: APP_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

/// The public presentation-mode enum defined in `app_package.mfb`, referenced
/// bare (`Mode`) like every other builtin type. Wrapper over `APP.types`.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    APP.types.iter().any(|ty| ty.name == name)
}

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_app_call(name: &str) -> bool {
    DefaultResolver::contains(&APP, name)
}

// `call_param_names`, `argument_types`, and `expected_arguments` return
// `&'static` borrowed shapes that `DefaultResolver` derives as owned
// (`Vec`/`String`); a runtime conversion cannot yield `&'static`, and the
// consumers (the syntaxcheck `BUILTIN_PACKAGES` table, IR lowering) require the
// borrowed type. So these three stay as static literals for now, PINNED equal to
// `APP` by `parity_matches_descriptor` below. BB removes them once it moves the
// consumers onto the owned descriptor API.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        GET_MODE => &[],
        SET_MODE => &[&["mode"]],
        _ => return None,
    };
    Some(params)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    DefaultResolver::return_type_name(&APP, name)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    DefaultResolver::resolve_call(&APP, name, arg_types).map(|return_type| ResolvedCall {
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
    DefaultResolver::arity(&APP, name)
}

super::package_source_glue!(
    "app",
    "<builtin-app>",
    "builtins/app.mfb",
    include_str!("app_package.mfb")
);

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

    // plan-72-B migration gate: prove the `APP` descriptor reproduces every
    // legacy helper answer for every `app.*` name — membership, arity, param
    // names, return type, expected arguments, argument types — and pins the three
    // static wrappers (`call_param_names`, `argument_types`, `expected_arguments`)
    // equal to the descriptor-derived forms. Keep until the legacy helpers are
    // deleted in plan-72-BB. See `builtins::descriptor::parity`.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let legacy = parity::LegacySet {
            is_call: &is_app_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &call_return_type_name,
            expected_arguments: &|name| expected_arguments(name).map(str::to_string),
            param_name_overloads: None,
            argument_types: Some(&|name| {
                argument_types(name).map(<[&str]>::to_vec)
            }),
            implementation_name: Some(&|_| None),
            default_padding: None,
            // `Mode` is an enum builtin type with no record fields.
            builtin_type_fields: Some(&|name| match name {
                "Mode" => Some(&[] as &'static [(&'static str, &'static str)]),
                _ => None,
            }),
        };
        parity::assert_parity(&APP, &[GET_MODE, SET_MODE, "app.nope"], &legacy, &[]);

        // The source companion type `Mode` is a builtin type name and its
        // companion injects on import (WhenImported).
        assert!(is_builtin_type("Mode"));
        assert!(!is_builtin_type("Console"));
        assert_eq!(
            APP.source.expect("app has a source").rule,
            InjectionRule::WhenImported
        );
        assert_eq!(APP.types.iter().map(|ty| ty.name).collect::<Vec<_>>(), vec!["Mode"]);

        // resolve_call parity across accepted and rejected argument shapes.
        assert_eq!(resolve_call(GET_MODE, &[]).unwrap().return_type, "Mode");
        assert_eq!(
            resolve_call(SET_MODE, &[String::from("Mode")])
                .unwrap()
                .return_type,
            "Nothing"
        );
        assert!(resolve_call(SET_MODE, &[String::from("Integer")]).is_none());
        assert!(resolve_call(GET_MODE, &[String::from("Mode")]).is_none());
    }
}
