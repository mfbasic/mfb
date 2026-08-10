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
        doc_into: "",
        doc_desc: "",
        errors: &[],
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
        doc_into: "",
        doc_desc: "",
        errors: &[],
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

super::package_source_glue!(
    "app",
    "<builtin-app>",
    "builtins/app.mfb",
    include_str!("app_package.mfb")
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_types_machine_table() {
        // bug-340 A1: the machine-readable positional signature IR lowering reads.
        assert_eq!(argument_types(SET_MODE), Some(&["Mode"][..]));
        // getMode takes no arguments -> nothing to type.
        assert_eq!(argument_types(GET_MODE), None);
        assert_eq!(argument_types("app.nope"), None);
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

    #[test]
    fn is_builtin_type_recognizes_mode() {
        assert!(is_builtin_type("Mode"));
        assert!(!is_builtin_type("Console"));
        assert!(!is_builtin_type("Nope"));
    }
}
