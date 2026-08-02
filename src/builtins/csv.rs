
use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, DefaultResolver,
    Implementation, InjectionRule, Parameter, ParameterType, ReturnType,
};

const PARSE: &str = "csv.parse";
const STRINGIFY: &str = "csv.stringify";
const INTERNAL_PARSE: &str = "__csv_parse";
const INTERNAL_STRINGIFY: &str = "__csv_stringify";

const GRID_TYPE: &str = "List OF List OF String";

// plan-72-G: `CSV` is the descriptor authority. Both functions are data-shaped
// with a fixed implementation rewrite (`__csv_*`), so no resolver is needed — the
// plan's "1 custom-resolver helper" is really a fixed `Implementation::Rewrite`
// map. Lowering is a runtime helper (source-package body). `WhenImported` source.
const CSV_FUNCTIONS: &[BuiltinFunction] = &[
    BuiltinFunction {
        name: PARSE,
        doc_slug: "parse",
        overloads: &[BuiltinOverload {
            params: &[Parameter {
                name: "value",
                aliases: &["text"],
                ty: ParameterType::Named("String"),
                default: super::descriptor::DefaultValue::None,
            }],
            return_type: ReturnType::Fixed(GRID_TYPE),
        }],
        implementation: Implementation::Rewrite(INTERNAL_PARSE),
        lowering: super::descriptor::Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
    BuiltinFunction {
        name: STRINGIFY,
        doc_slug: "stringify",
        overloads: &[BuiltinOverload {
            params: &[Parameter::required("value", GRID_TYPE)],
            return_type: ReturnType::Fixed("String"),
        }],
        implementation: Implementation::Rewrite(INTERNAL_STRINGIFY),
        lowering: super::descriptor::Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
];

pub(crate) static CSV: BuiltinModule = BuiltinModule {
    name: "csv",
    functions: CSV_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

pub(crate) fn is_csv_call(name: &str) -> bool {
    DefaultResolver::contains(&CSV, name)
}

// `call_param_names` and `expected_arguments` return `&'static` borrowed shapes
// the owned `DefaultResolver` cannot produce, so they stay static, PINNED equal
// to `CSV` by the parity test until plan-72-BB.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        PARSE => Some(&[&["value", "text"]]),
        STRINGIFY => Some(&[&["value"]]),
        _ => None,
    }
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some("String"),
        STRINGIFY => Some(GRID_TYPE),
        _ => None,
    }
}

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    DefaultResolver::implementation_name(&CSV, name)
}

super::package_source_glue!(
    "csv",
    "<builtin-csv>",
    "builtins/csv.mfb",
    include_str!("csv_package.mfb")
);

#[cfg(test)]
mod tests {
    use super::*;

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
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
