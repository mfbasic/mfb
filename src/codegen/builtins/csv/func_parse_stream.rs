//! `csv::parseStream` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:parseStream@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};

const INTRO: &str = r#"Open a streaming reader over UTF-8 CSV text."#;

const DESC: &str = r#"`csv::parseStream` returns a `CsvReader` — a value holding the decoded input and a
scan cursor — without parsing any rows yet. Each subsequent `csv::readRow` parses
exactly one record and returns it with the reader advanced, so a document is
processed one row at a time and the whole `List OF List OF String` grid is never
materialized. The rows a `parseStream`/`readRow` loop yields are identical to
`csv::parse(value)`.

The optional `delimiter` and `quote` select the input dialect exactly as for
`csv::parse` (defaults `,` and `"`); each must be a non-empty single character.
The output-only dialect option (`newline`) does not apply to reading.

The argument may also be supplied by the name `text`. `csv::parseStream` does not
mutate `value` and has no side effects."#;

const EX: &str = r#"Process a CSV document row by row without building the whole grid:

```
IMPORT csv
IMPORT collections
IMPORT io

SUB main()
  MUT row AS CsvRow = csv::readRow(csv::parseStream("a,b\nc,d"))
  WHILE row.done = FALSE
    io::print(collections::get(row.fields, 0))
    row = csv::readRow(row.reader)
  END WHILE
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __csv_parseStream(value AS String, delimiter AS String, quote AS String) AS CsvReader
  LET chars AS List OF Integer = encoding::utf32Encode(value)
  RETURN CsvReader[chars, len(chars), 0, __csv_firstCode(delimiter), __csv_firstCode(quote)]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "parseStream",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The UTF-8 CSV text to stream.",
                    aliases: &["text"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "delimiter",
                    desc: "The single character that separates fields. Defaults to ,.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: super::DEFAULT_DELIMITER,
                    },
                },
                Parameter {
                    name: "quote",
                    desc: "The single character that wraps a field and, doubled, escapes itself. Defaults to \".",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: super::DEFAULT_QUOTE,
                    },
                },
            ],
            return_type: ParameterType::Named("CsvReader"),
            errors: vec![],
            body: Body::mfb(FUNC_BODY, "__csv_parseStream"),
        }],
    });
}
