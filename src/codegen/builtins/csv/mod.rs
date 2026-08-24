//! Package: csv
//! Type: Pure MFBasic

use crate::codegen::registry::{RecordProp, Registry, RegistryPackage, RegistryRecord};
use crate::types::ParameterType;

mod func_parse;
mod func_parse_stream;
mod func_read_row;
mod func_stringify;

mod helper_decode_range;
mod helper_encode_field;
mod helper_field_value;
mod helper_first_code;
mod helper_is_doubled_quote;
mod helper_needs_quote;
mod helper_quote_field;
mod helper_separator_length;
mod helper_stringify_row;

/// RFC-4180 dialect defaults, injected as raw String literals when an optional
/// dialect argument is omitted (the `expr` of a `Fill` is the literal value).
pub(crate) const DEFAULT_DELIMITER: &str = ",";
pub(crate) const DEFAULT_QUOTE: &str = "\"";
pub(crate) const DEFAULT_NEWLINE: &str = "\n";

const INTRO: &str = r#"Parse and serialize CSV text as a grid of String cells"#;

const DESC: &str = r#"The `csv` package converts between CSV text and a grid of rows of String cells.
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

/// Register the `csv` package on the registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("csv", INTRO, DESC);

    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    // plan-77 C4: the streaming reader/row records (EXPORT so callers can name them).
    pkg.add_record(RegistryRecord {
        name: "CsvReader",
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "chars",
                ty: ParameterType::list_of(ParameterType::Integer),
                description: "The decoded input as Unicode scalars.",
            },
            RecordProp {
                name: "count",
                ty: ParameterType::Integer,
                description: "The scalar count.",
            },
            RecordProp {
                name: "index",
                ty: ParameterType::Integer,
                description: "The scan cursor.",
            },
            RecordProp {
                name: "delimCode",
                ty: ParameterType::Integer,
                description: "The field-delimiter scalar.",
            },
            RecordProp {
                name: "quoteCode",
                ty: ParameterType::Integer,
                description: "The quote-character scalar.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "CsvRow",
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "fields",
                ty: ParameterType::list_of(ParameterType::String),
                description: "The record's cells.",
            },
            RecordProp {
                name: "reader",
                ty: ParameterType::named("CsvReader"),
                description: "The reader advanced past this record.",
            },
            RecordProp {
                name: "done",
                ty: ParameterType::Boolean,
                description: "TRUE when the reader was already at end of input.",
            },
        ],
    });

    // The shared private `__csv_*` helpers the member bodies call. Each lives in
    // its own `helper_*.rs` and registers via `add_helper`; they render (in this
    // order) in the helper section of the assembled source, before the member
    // bodies. Order is preserved from the old single `package.mfb` blob so the
    // compiled `.ncode` stays byte-identical.
    helper_field_value::register(&mut pkg);
    helper_decode_range::register(&mut pkg);
    helper_is_doubled_quote::register(&mut pkg);
    helper_first_code::register(&mut pkg);
    helper_separator_length::register(&mut pkg);
    helper_stringify_row::register(&mut pkg);
    helper_encode_field::register(&mut pkg);
    helper_needs_quote::register(&mut pkg);
    helper_quote_field::register(&mut pkg);

    func_parse::register(&mut pkg);
    func_stringify::register(&mut pkg);
    func_parse_stream::register(&mut pkg);
    func_read_row::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn csv_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("csv").expect("csv package");
        assert_eq!(pkg.functions().len(), 4);
        // The two EXPORT records are visible to the generic type query.
        assert!(registry().is_builtin_type("CsvReader"));
        assert!(registry().is_builtin_type("CsvRow"));
    }

    #[test]
    fn generic_dispatch_reaches_csv() {
        assert!(registry().is_member("csv.parse"));
        assert!(!registry().is_member("csv.nope"));
        assert_eq!(
            registry::call_return_type("csv.parse").as_deref(),
            Some("List OF List OF String")
        );
        assert_eq!(
            registry::rewrite_target("csv.parse", &[]),
            Some("__csv_parse")
        );
        // The irregular pairing the public name cannot derive.
        assert_eq!(
            registry::rewrite_target("csv.readRow", &[]),
            Some("__csv_next")
        );
        assert_eq!(registry().arity("csv.parse"), Some((1, 3)));
        assert_eq!(registry().arity("csv.readRow"), Some((1, 1)));
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry().resolve_package("csv").expect("csv").get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-csv>"),
            "builtins/csv.mfb",
            &source,
        )
        .expect("reassembled csv source parses");
    }
}
