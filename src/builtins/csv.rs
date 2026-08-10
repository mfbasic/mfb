use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, BuiltinType,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Parameter, ParameterType,
    ReturnType, TypeKind,
};

const PARSE: &str = "csv.parse";
const STRINGIFY: &str = "csv.stringify";
const PARSE_STREAM: &str = "csv.parseStream";
// Named `readRow`, not `next` — `next` collides with the `NEXT` loop keyword.
const NEXT: &str = "csv.readRow";
const INTERNAL_PARSE: &str = "__csv_parse";
const INTERNAL_STRINGIFY: &str = "__csv_stringify";
const INTERNAL_PARSE_STREAM: &str = "__csv_parseStream";
const INTERNAL_NEXT: &str = "__csv_next";

const GRID_TYPE: &str = "List OF List OF String";
// plan-77 C4: streaming reader/row record types (declared in csv_package.mfb as
// `EXPORT TYPE`). Referenced bare, like datetime's `Instant`.
const READER_TYPE: &str = "CsvReader";
const ROW_TYPE: &str = "CsvRow";

// plan-77 C3: optional trailing dialect parameters, default-PADDED at IR lowering
// (like datetime's `time`), so `csv::parse(s)` / `csv::stringify(g)` keep their
// one-argument shape while `csv::parse(s, ";", "'")` overrides the dialect. Each
// `expr` is the RFC-4180 default as a source String literal.
const fn csv_opt(name: &'static str, default_expr: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named("String"),
        default: DefaultValue::Fill {
            type_name: "String",
            expr: default_expr,
        },
    }
}

// The `expr` of a String `Fill` is injected as the const's RAW value (ir/lower.rs
// builds `IrValue::Const { type_: "String", value: expr }`), so these are the
// literal characters, not quoted source tokens.
const DEFAULT_DELIMITER: &str = ",";
const DEFAULT_QUOTE: &str = "\"";
const DEFAULT_NEWLINE: &str = "\n";

const P_PARSE: &[Parameter] = &[
    Parameter {
        name: "value",
        aliases: &["text"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
    csv_opt("delimiter", DEFAULT_DELIMITER),
    csv_opt("quote", DEFAULT_QUOTE),
];

const P_STRINGIFY: &[Parameter] = &[
    Parameter::required("value", GRID_TYPE),
    csv_opt("delimiter", DEFAULT_DELIMITER),
    csv_opt("quote", DEFAULT_QUOTE),
    csv_opt("newline", DEFAULT_NEWLINE),
];

// parseStream takes the same input + optional dialect as parse.
const P_PARSE_STREAM: &[Parameter] = P_PARSE;
const P_NEXT: &[Parameter] = &[Parameter::required("reader", READER_TYPE)];

const CSV_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: READER_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: ROW_TYPE,
        kind: TypeKind::Record,
        fields: &[],
    },
];

// plan-72-G: `CSV` is the descriptor authority. Both functions are data-shaped
// with a fixed implementation rewrite (`__csv_*`), so no resolver is needed — the
// plan's "1 custom-resolver helper" is really a fixed `Implementation::Rewrite`
// map. Lowering is a runtime helper (source-package body). `WhenImported` source.
const CSV_FUNCTIONS: &[BuiltinFunction] = &[
    BuiltinFunction {
        name: PARSE,
        doc_slug: "parse",
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: P_PARSE,
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
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: P_STRINGIFY,
            return_type: ReturnType::Fixed("String"),
        }],
        implementation: Implementation::Rewrite(INTERNAL_STRINGIFY),
        lowering: super::descriptor::Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
    BuiltinFunction {
        name: PARSE_STREAM,
        doc_slug: "parseStream",
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: P_PARSE_STREAM,
            return_type: ReturnType::Fixed(READER_TYPE),
        }],
        implementation: Implementation::Rewrite(INTERNAL_PARSE_STREAM),
        lowering: super::descriptor::Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
    BuiltinFunction {
        name: NEXT,
        doc_slug: "readRow",
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: P_NEXT,
            return_type: ReturnType::Fixed(ROW_TYPE),
        }],
        implementation: Implementation::Rewrite(INTERNAL_NEXT),
        lowering: super::descriptor::Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    },
];

pub(crate) static CSV: BuiltinModule = BuiltinModule {
    name: "csv",
    doc_intro: "",
    doc_desc: "",
    functions: CSV_FUNCTIONS,
    types: CSV_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

pub(crate) fn is_csv_call(name: &str) -> bool {
    DefaultResolver::contains(&CSV, name)
}

// plan-77 C4: the streaming reader/row record types (referenced bare, or
// qualified as `csv.CsvReader`/`csv.CsvRow` via `qualified_builtin_type`).
pub(crate) fn is_builtin_type(name: &str) -> bool {
    CSV.types.iter().any(|ty| ty.name == name)
}

// `call_param_names` and `expected_arguments` return `&'static` borrowed shapes
// the owned `DefaultResolver` cannot produce, so they stay static, PINNED equal
// to `CSV` by the parity test until plan-72-BB.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        PARSE => Some(&[&["value", "text"], &["delimiter"], &["quote"]]),
        STRINGIFY => Some(&[&["value"], &["delimiter"], &["quote"], &["newline"]]),
        PARSE_STREAM => Some(&[&["value", "text"], &["delimiter"], &["quote"]]),
        NEXT => Some(&[&["reader"]]),
        _ => None,
    }
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some("String"),
        STRINGIFY => Some(GRID_TYPE),
        PARSE_STREAM => Some("String"),
        NEXT => Some(READER_TYPE),
        _ => None,
    }
}

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    DefaultResolver::implementation_name(&CSV, name)
}

// plan-77 C3: pad the omitted trailing dialect arguments with the RFC-4180
// defaults, so `#csv_parse`/`#csv_stringify` always receive their full arity.
// The order/exprs mirror `P_PARSE`/`P_STRINGIFY`'s `Fill` params (both begin with
// the one required `value`, hence `provided - 1` optionals already supplied).
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const PARSE_DEFAULTS: &[(&str, &str)] =
        &[("String", DEFAULT_DELIMITER), ("String", DEFAULT_QUOTE)];
    const STRINGIFY_DEFAULTS: &[(&str, &str)] = &[
        ("String", DEFAULT_DELIMITER),
        ("String", DEFAULT_QUOTE),
        ("String", DEFAULT_NEWLINE),
    ];
    match name {
        PARSE | PARSE_STREAM => {
            &PARSE_DEFAULTS[provided.saturating_sub(1).min(PARSE_DEFAULTS.len())..]
        }
        STRINGIFY => {
            &STRINGIFY_DEFAULTS[provided.saturating_sub(1).min(STRINGIFY_DEFAULTS.len())..]
        }
        _ => &[],
    }
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
        assert_eq!(
            call_param_names(PARSE),
            Some(&[&["value", "text"][..], &["delimiter"][..], &["quote"][..]][..])
        );
        assert_eq!(
            call_param_names(STRINGIFY),
            Some(
                &[
                    &["value"][..],
                    &["delimiter"][..],
                    &["quote"][..],
                    &["newline"][..]
                ][..]
            )
        );
        assert_eq!(
            call_param_names(PARSE_STREAM),
            Some(&[&["value", "text"][..], &["delimiter"][..], &["quote"][..]][..])
        );
        assert_eq!(call_param_names(NEXT), Some(&[&["reader"][..]][..]));
        assert_eq!(call_param_names("csv.other"), None);
    }

    #[test]
    fn streaming_types_are_registered() {
        assert!(is_builtin_type(READER_TYPE));
        assert!(is_builtin_type(ROW_TYPE));
        assert!(!is_builtin_type("Nope"));
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
    fn csv_opt_builds_a_fill_defaulted_string_parameter() {
        // `csv_opt` is a `const fn` used only in `const` table context, so it is
        // const-evaluated and shows as uncovered; exercise it at runtime here.
        // `black_box` the arguments so the call cannot be folded back to a const.
        use std::hint::black_box;
        let p = csv_opt(black_box("delimiter"), black_box(DEFAULT_DELIMITER));
        assert_eq!(p.name, "delimiter");
        assert!(p.aliases.is_empty());
        assert!(matches!(p.ty, ParameterType::Named("String")));
        match p.default {
            DefaultValue::Fill { type_name, expr } => {
                assert_eq!(type_name, "String");
                assert_eq!(expr, ",");
            }
            other => panic!("expected a String Fill default, got {other:?}"),
        }
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
