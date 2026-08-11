//! Built-in `csv` package (plan-72-G / plan-77), migrated into the codegen layer
//! (planning/migrate.md). Both parse/stringify and the streaming reader are
//! source-backed: each member's `__csv_*` body lives in its `func_*.rs` as
//! `Implementation::Mfb` and is spliced into the injected package source by
//! `assembled_source()` in place of a `'@@MFB_BODY:<slug>@@` marker; the private
//! helpers and the `EXPORT TYPE CsvReader`/`CsvRow` declarations stay in
//! `package.mfb`. csv is a concrete package rewritten in IR lowering, so the
//! `__csv_*` rewrite target comes from the explicit `IMPL_NAMES` table (note
//! `readRow` → `__csv_next`, not `__csv_readRow`).

use crate::codegen::registry::{
    BuiltinFunction, BuiltinModule, BuiltinSource, BuiltinType, DefaultResolver, DefaultValue,
    Implementation, InjectionRule, Parameter, ParameterType, TypeKind,
};

mod func_parse;
mod func_parse_stream;
mod func_read_row;
mod func_stringify;

const PARSE: &str = "csv.parse";
const STRINGIFY: &str = "csv.stringify";
const PARSE_STREAM: &str = "csv.parseStream";
// Named `readRow`, not `next` — `next` collides with the `NEXT` loop keyword.
const NEXT: &str = "csv.readRow";
const INTERNAL_PARSE: &str = "__csv_parse";
const INTERNAL_STRINGIFY: &str = "__csv_stringify";
const INTERNAL_PARSE_STREAM: &str = "__csv_parseStream";
const INTERNAL_NEXT: &str = "__csv_next";

pub(super) const GRID_TYPE: &str = "List OF List OF String";
// plan-77 C4: streaming reader/row record types (declared in package.mfb as
// `EXPORT TYPE`). Referenced bare, like datetime's `Instant`.
pub(super) const READER_TYPE: &str = "CsvReader";
pub(super) const ROW_TYPE: &str = "CsvRow";

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

pub(super) const P_PARSE: &[Parameter] = &[
    Parameter {
        name: "value",
        aliases: &["text"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
    csv_opt("delimiter", DEFAULT_DELIMITER),
    csv_opt("quote", DEFAULT_QUOTE),
];

pub(super) const P_STRINGIFY: &[Parameter] = &[
    Parameter::required("value", GRID_TYPE),
    csv_opt("delimiter", DEFAULT_DELIMITER),
    csv_opt("quote", DEFAULT_QUOTE),
    csv_opt("newline", DEFAULT_NEWLINE),
];

// parseStream takes the same input + optional dialect as parse.
pub(super) const P_PARSE_STREAM: &[Parameter] = P_PARSE;
pub(super) const P_NEXT: &[Parameter] = &[Parameter::required("reader", READER_TYPE)];

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

// plan-72-G: `CSV` is the descriptor authority. Each member owns its source body
// in its `func_*.rs` (`Implementation::Mfb`); a call rewrites to the internal
// `__csv_*` name via `IMPL_NAMES` at IR lowering. `WhenImported` source.
const CSV_FUNCTIONS: &[BuiltinFunction] = &[
    func_parse::PARSE,
    func_stringify::STRINGIFY,
    func_parse_stream::PARSE_STREAM,
    func_read_row::READ_ROW,
];

const MODULE_INTRO: &str = r#"Parse and serialize CSV text as a grid of String cells"#;
const MODULE_DESC: &str = r#"The `csv` package converts between CSV text and a grid of rows of String cells.
`csv::parse` turns a UTF-8 `String` holding CSV text into a
`List OF List OF String`, and `csv::stringify` renders such a grid back into CSV
text. `csv` is a built-in package: `IMPORT csv` needs no manifest dependency.

A whole-document CSV is exactly a `List OF List OF String`: an ordered list of
rows, each an ordered list of String cells. The parsed grid composes directly
with the `collections` package and `FOR EACH`; cells are read positionally with
`collections::get`; there is no header concept — every parsed line is an ordinary
row.

For large inputs there is a streaming alternative that never materializes the
whole grid: `csv::parseStream` returns a `CsvReader` holding the input and a scan
cursor, and each `csv::readRow` parses exactly one record and returns a `CsvRow`
(`fields AS List OF String`, `reader AS CsvReader` advanced past the record, and
`done AS Boolean`). A caller loops `WHILE row.done = FALSE` (see `mfb man csv
readRow`). The rows are identical to `csv::parse`'s.

Cells are plain Strings. There is no type inference and no null: `42`, `true`,
and an empty field are just the Strings `"42"`, `"true"`, and `""`. Callers that
want numbers convert explicitly with `toFloat` or `toInteger`. Rows are not
required to be rectangular: `csv::parse` preserves whatever field count each row
had.

The dialect is RFC-4180-aligned by default, but the field delimiter and quote
character are configurable: `parse`/`parseStream` take optional `delimiter` and
`quote`, and `stringify` also takes an optional output `newline`, each defaulting
to `,`, `"`, and LF. On input, a record separator is a line feed (LF) or a
carriage-return/line-feed pair (CRLF); a bare CR not followed by LF is ordinary
data. A field may be wrapped in the quote character, inside which a literal quote
is written by doubling it and delimiters, CR, and LF are ordinary data.
Whitespace is significant and never trimmed. A single trailing record separator does not
create an empty final row, but two consecutive separators do produce an empty
row in the middle. Empty input parses to zero rows.

`csv::stringify` renders deterministically: rows are joined with a single LF
with no trailing newline, fields within a row are joined with a comma, and a
field is quoted only when it contains a comma, a double quote, a CR, or an LF.
For any grid `x`, `csv::parse(csv::stringify(x))` yields a grid whose cells
equal those of `x`, except that a trailing empty row produced only by separator
placement is not reintroduced and a CRLF separator is normalized to LF."#;

pub(crate) static CSV: BuiltinModule = BuiltinModule {
    name: "csv",
    doc_intro: MODULE_INTRO,
    doc_desc: MODULE_DESC,
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

/// The internal `__csv_*` symbol each public member rewrites to during IR
/// lowering. The members carry `Implementation::Mfb` (whose descriptor
/// `implementation_name` is `None`), so the rewrite target is provided here — note
/// `csv.readRow` → `__csv_next`, an irregular slug/internal pairing a derivation
/// could not reproduce.
const IMPL_NAMES: &[(&str, &str)] = &[
    (PARSE, INTERNAL_PARSE),
    (STRINGIFY, INTERNAL_STRINGIFY),
    (PARSE_STREAM, INTERNAL_PARSE_STREAM),
    (NEXT, INTERNAL_NEXT),
];

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    IMPL_NAMES
        .iter()
        .find(|(public, _)| *public == name)
        .map(|(_, internal)| *internal)
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

/// Synthetic path label / doc path for the injected csv source. Preserved
/// byte-for-byte from the pre-migration `package_source_glue!` invocation so the
/// injected AST is unchanged.
const SOURCE_LABEL: &str = "<builtin-csv>";
const SOURCE_DOC: &str = "builtins/csv.mfb";

/// Parses the built-in `csv` package source (the `package.mfb` companion plus
/// every `Implementation::Mfb` member body, spliced in by `assembled_source`).
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new(SOURCE_LABEL),
        SOURCE_DOC,
        &assembled_source(),
    )
}

/// The `csv` package source: the `package.mfb` companion (helpers + `EXPORT TYPE`
/// decls) with each member's `FUNC __csv_* ... END FUNC` body spliced in for its
/// `'@@MFB_BODY:<slug>@@` marker at the body's original position. Splicing in
/// place keeps every other line's number unchanged, so the injected AST — and
/// every derived golden — is byte-identical to the pre-migration companion.
fn assembled_source() -> String {
    let mut source = String::from(include_str!("package.mfb"));
    for func in CSV_FUNCTIONS {
        if let Implementation::Mfb { body, .. } = func.implementation {
            let marker = format!("'@@MFB_BODY:{}@@", func.doc_slug);
            debug_assert!(
                source.contains(&marker),
                "csv package.mfb is missing the '{marker}' body marker",
            );
            source = source.replacen(&marker, body, 1);
        }
    }
    source
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
        // The irregular slug/internal pairing: readRow → __csv_next.
        assert_eq!(implementation_name(NEXT), Some(INTERNAL_NEXT));
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
