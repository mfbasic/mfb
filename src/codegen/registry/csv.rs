//! The built-in `csv` package, authored on the clean-room registry — the first
//! real package migrated off `target::shared::registry` (see the map in the session
//! notes / `planning/todo.md`).
//!
//! csv is a source-backed package: every member owns a `FUNC __csv_* … END FUNC`
//! body ([`Body::mfb`]) that a call rewrites to. The private helpers and the two
//! `EXPORT TYPE` records live on the package as helper functions and records, and
//! [`RegistryPackage::get_mfb`] reassembles the whole injectable source. Bodies are
//! byte-significant (2-space indent → `.ncode` columns) and are copied verbatim from
//! the pre-migration `func_*.rs` / `package.mfb`.

use super::{Body, DefaultValue, Implementation, Lowering, Parameter, RecordProp, Registry};

/// RFC-4180 dialect defaults, injected as raw String literals when the optional
/// dialect arguments are omitted (the `expr` of a `Fill` is the literal value).
const DEFAULT_DELIMITER: &str = ",";
const DEFAULT_QUOTE: &str = "\"";
const DEFAULT_NEWLINE: &str = "\n";

/// A required String parameter (with optional keyword aliases).
fn required(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

/// An optional String parameter, default-padded with `expr` when omitted.
fn opt(name: &'static str, expr: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: "String",
        default: DefaultValue::Fill {
            type_name: "String",
            expr,
        },
    }
}

fn string_impl(
    params: Vec<Parameter>,
    return_type: &'static str,
    body: &'static str,
    rewrite: &'static str,
) -> Implementation {
    Implementation {
        params,
        return_type,
        errors: vec![],
        lowering: Lowering::Helper,
        body: Body::mfb(body, rewrite),
    }
}

/// The shared private helpers (`__csv_*`) the member bodies call — copied verbatim
/// from `package.mfb`.
#[rustfmt::skip]
const HELPERS: &str =
r#"FUNC __csv_fieldValue(chars AS List OF Integer, buf AS List OF Integer, wasQuoted AS Boolean, fieldStart AS Integer, index AS Integer) AS String
  IF wasQuoted THEN
    RETURN encoding::utf32Decode(buf)
  END IF
  RETURN __csv_decodeRange(chars, fieldStart, index)
END FUNC

FUNC __csv_decodeRange(chars AS List OF Integer, startIndex AS Integer, endIndex AS Integer) AS String
  MUT out AS String = ""
  MUT i AS Integer = startIndex
  WHILE i < endIndex
    LET cp AS Integer = collections::get(chars, i)
    IF cp < 0 OR cp > 1114111 THEN
      FAIL error(77050003, "invalid code point")
    END IF
    IF cp >= 55296 AND cp <= 57343 THEN
      FAIL error(77050003, "surrogate code point")
    END IF
    out = out & __encoding_fromCodepoint(cp)
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __csv_isDoubledQuote(chars AS List OF Integer, count AS Integer, index AS Integer, quoteCode AS Integer) AS Boolean
  IF index + 1 >= count THEN
    RETURN FALSE
  END IF
  IF collections::get(chars, index + 1) = quoteCode THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC

FUNC __csv_firstCode(s AS String) AS Integer
  LET codes AS List OF Integer = encoding::utf32Encode(s)
  IF len(codes) = 0 THEN
    FAIL error(77050003, "csv: dialect delimiter/quote must be a non-empty character")
  END IF
  RETURN collections::get(codes, 0)
END FUNC

FUNC __csv_separatorLength(chars AS List OF Integer, count AS Integer, index AS Integer) AS Integer
  LET ch AS Integer = collections::get(chars, index)
  IF ch = 10 THEN
    RETURN 1
  END IF
  IF ch = 13 THEN
    IF index + 1 < count THEN
      IF collections::get(chars, index + 1) = 10 THEN
        RETURN 2
      END IF
    END IF
  END IF
  RETURN 0
END FUNC

FUNC __csv_stringifyRow(row AS List OF String, delimiter AS String, quote AS String) AS String
  MUT out AS String = ""
  MUT firstField AS Boolean = TRUE
  FOR EACH field IN row
    IF firstField THEN
      firstField = FALSE
    ELSE
      out = out & delimiter
    END IF
    out = out & __csv_encodeField(field, delimiter, quote)
  NEXT
  RETURN out
END FUNC

FUNC __csv_encodeField(field AS String, delimiter AS String, quote AS String) AS String
  IF __csv_needsQuote(field, delimiter, quote) THEN
    RETURN __csv_quoteField(field, quote)
  END IF
  RETURN field
END FUNC

FUNC __csv_needsQuote(field AS String, delimiter AS String, quote AS String) AS Boolean
  IF strings::contains(field, delimiter) THEN
    RETURN TRUE
  END IF
  IF strings::contains(field, quote) THEN
    RETURN TRUE
  END IF
  IF strings::contains(field, "\r") THEN
    RETURN TRUE
  END IF
  IF strings::contains(field, "\n") THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC

FUNC __csv_quoteField(field AS String, quote AS String) AS String
  RETURN quote & strings::replace(field, quote, quote & quote) & quote
END FUNC"#;

#[rustfmt::skip]
const BODY_PARSE: &str =
r#"FUNC __csv_parse(value AS String, delimiter AS String, quote AS String) AS List OF List OF String
  LET delimCode AS Integer = __csv_firstCode(delimiter)
  LET quoteCode AS Integer = __csv_firstCode(quote)
  LET chars AS List OF Integer = encoding::utf32Encode(value)
  LET count AS Integer = len(chars)
  MUT rows AS List OF List OF String = []
  MUT row AS List OF String = []
  MUT fieldBuf AS List OF Integer = []
  MUT fieldStart AS Integer = 0
  MUT index AS Integer = 0
  MUT inQuotes AS Boolean = FALSE
  MUT fieldStarted AS Boolean = FALSE
  MUT wasQuoted AS Boolean = FALSE
  MUT recordPending AS Boolean = FALSE

  WHILE index < count
    LET ch AS Integer = collections::get(chars, index)
    IF inQuotes THEN
      IF ch = quoteCode THEN
        IF __csv_isDoubledQuote(chars, count, index, quoteCode) THEN
          fieldBuf = collections::append(fieldBuf, quoteCode)
          index = index + 2
        ELSE
          inQuotes = FALSE
          wasQuoted = TRUE
          index = index + 1
        END IF
      ELSE
        fieldBuf = collections::append(fieldBuf, ch)
        index = index + 1
      END IF
    ELSEIF __csv_separatorLength(chars, count, index) > 0 THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      rows = collections::append(rows, row)
      row = []
      fieldBuf = []
      fieldStarted = FALSE
      wasQuoted = FALSE
      recordPending = FALSE
      index = index + __csv_separatorLength(chars, count, index)
      fieldStart = index
    ELSEIF ch = delimCode THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      fieldBuf = []
      fieldStarted = FALSE
      wasQuoted = FALSE
      recordPending = TRUE
      index = index + 1
      fieldStart = index
    ELSEIF wasQuoted THEN
      FAIL error(77050003, "invalid CSV format")
    ELSEIF ch = quoteCode AND fieldStarted = FALSE THEN
      inQuotes = TRUE
      fieldStarted = TRUE
      index = index + 1
    ELSE
      fieldStarted = TRUE
      index = index + 1
    END IF
  END WHILE

  IF inQuotes THEN
    FAIL error(77050003, "invalid CSV format")
  END IF

  IF fieldStarted OR recordPending OR wasQuoted THEN
    row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
    rows = collections::append(rows, row)
  END IF

  RETURN rows
END FUNC"#;

#[rustfmt::skip]
const BODY_STRINGIFY: &str =
r#"FUNC __csv_stringify(value AS List OF List OF String, delimiter AS String, quote AS String, newline AS String) AS String
  MUT out AS String = ""
  MUT firstRow AS Boolean = TRUE
  FOR EACH row IN value
    IF firstRow THEN
      firstRow = FALSE
    ELSE
      out = out & newline
    END IF
    out = out & __csv_stringifyRow(row, delimiter, quote)
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const BODY_PARSE_STREAM: &str =
r#"FUNC __csv_parseStream(value AS String, delimiter AS String, quote AS String) AS CsvReader
  LET chars AS List OF Integer = encoding::utf32Encode(value)
  RETURN CsvReader[chars, len(chars), 0, __csv_firstCode(delimiter), __csv_firstCode(quote)]
END FUNC"#;

#[rustfmt::skip]
const BODY_NEXT: &str =
r#"FUNC __csv_next(reader AS CsvReader) AS CsvRow
  LET chars AS List OF Integer = reader.chars
  LET count AS Integer = reader.count
  LET delimCode AS Integer = reader.delimCode
  LET quoteCode AS Integer = reader.quoteCode
  MUT index AS Integer = reader.index
  IF index >= count THEN
    RETURN CsvRow[[], reader, TRUE]
  END IF
  MUT row AS List OF String = []
  MUT fieldBuf AS List OF Integer = []
  MUT fieldStart AS Integer = index
  MUT inQuotes AS Boolean = FALSE
  MUT fieldStarted AS Boolean = FALSE
  MUT wasQuoted AS Boolean = FALSE
  WHILE index < count
    LET ch AS Integer = collections::get(chars, index)
    IF inQuotes THEN
      IF ch = quoteCode THEN
        IF __csv_isDoubledQuote(chars, count, index, quoteCode) THEN
          fieldBuf = collections::append(fieldBuf, quoteCode)
          index = index + 2
        ELSE
          inQuotes = FALSE
          wasQuoted = TRUE
          index = index + 1
        END IF
      ELSE
        fieldBuf = collections::append(fieldBuf, ch)
        index = index + 1
      END IF
    ELSEIF __csv_separatorLength(chars, count, index) > 0 THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      index = index + __csv_separatorLength(chars, count, index)
      RETURN CsvRow[row, CsvReader[chars, count, index, delimCode, quoteCode], FALSE]
    ELSEIF ch = delimCode THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      fieldBuf = []
      fieldStarted = FALSE
      wasQuoted = FALSE
      index = index + 1
      fieldStart = index
    ELSEIF wasQuoted THEN
      FAIL error(77050003, "invalid CSV format")
    ELSEIF ch = quoteCode AND fieldStarted = FALSE THEN
      inQuotes = TRUE
      fieldStarted = TRUE
      index = index + 1
    ELSE
      fieldStarted = TRUE
      index = index + 1
    END IF
  END WHILE
  IF inQuotes THEN
    FAIL error(77050003, "invalid CSV format")
  END IF
  row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
  RETURN CsvRow[row, CsvReader[chars, count, index, delimCode, quoteCode], FALSE]
END FUNC"#;

/// Register the `csv` package on the clean-room registry.
pub(super) fn register(r: &mut Registry) {
    let pkg = r.add_package(
        "csv",
        "Parse and serialize CSV text as a grid of String cells",
        "The `csv` package converts between CSV text and a grid of rows of String \
         cells: `csv::parse` / `csv::stringify` for the whole document, and \
         `csv::parseStream` / `csv::readRow` for row-by-row streaming.",
    );
    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    // plan-77 C4: the streaming reader/row records (EXPORT so callers can name them).
    pkg.add_record(
        "CsvReader",
        true,
        vec![
            RecordProp {
                name: "chars",
                ty: "List OF Integer",
                description: "The decoded input as Unicode scalars.",
            },
            RecordProp {
                name: "count",
                ty: "Integer",
                description: "The scalar count.",
            },
            RecordProp {
                name: "index",
                ty: "Integer",
                description: "The scan cursor.",
            },
            RecordProp {
                name: "delimCode",
                ty: "Integer",
                description: "The field-delimiter scalar.",
            },
            RecordProp {
                name: "quoteCode",
                ty: "Integer",
                description: "The quote-character scalar.",
            },
        ],
    );
    pkg.add_record(
        "CsvRow",
        true,
        vec![
            RecordProp {
                name: "fields",
                ty: "List OF String",
                description: "The record's cells.",
            },
            RecordProp {
                name: "reader",
                ty: "CsvReader",
                description: "The reader advanced past this record.",
            },
            RecordProp {
                name: "done",
                ty: "Boolean",
                description: "TRUE when the reader was already at end of input.",
            },
        ],
    );

    pkg.add_helper_functions(vec![HELPERS]);

    pkg.add_function(
        "parse",
        "Parse UTF-8 CSV text into a grid of String cells.",
        "Scan `value` into a `List OF List OF String`; optional RFC-4180 `delimiter` \
         and `quote` dialect.",
        "csv::parse(\"name,age\\nAda,36\")",
        vec![string_impl(
            vec![
                required("value", &["text"], "String"),
                opt("delimiter", DEFAULT_DELIMITER),
                opt("quote", DEFAULT_QUOTE),
            ],
            "List OF List OF String",
            BODY_PARSE,
            "__csv_parse",
        )],
    );

    pkg.add_function(
        "stringify",
        "Encode a grid of String cells as RFC-4180-aligned CSV text.",
        "Render a `List OF List OF String` to CSV text; optional `delimiter`, \
         `quote`, and `newline` dialect.",
        "csv::stringify([[\"a\", \"b\"], [\"c\", \"d\"]])",
        vec![string_impl(
            vec![
                required("value", &[], "List OF List OF String"),
                opt("delimiter", DEFAULT_DELIMITER),
                opt("quote", DEFAULT_QUOTE),
                opt("newline", DEFAULT_NEWLINE),
            ],
            "String",
            BODY_STRINGIFY,
            "__csv_stringify",
        )],
    );

    pkg.add_function(
        "parseStream",
        "Open a streaming reader over UTF-8 CSV text.",
        "Return a `CsvReader` over `value` without parsing rows yet; optional \
         `delimiter` and `quote` dialect.",
        "csv::parseStream(\"a,b\\nc,d\")",
        vec![string_impl(
            vec![
                required("value", &["text"], "String"),
                opt("delimiter", DEFAULT_DELIMITER),
                opt("quote", DEFAULT_QUOTE),
            ],
            "CsvReader",
            BODY_PARSE_STREAM,
            "__csv_parseStream",
        )],
    );

    // Named `readRow` (not `next` — collides with the NEXT loop keyword); rewrites
    // to the irregular internal `__csv_next`.
    pkg.add_function(
        "readRow",
        "Read the next record from a streaming CSV reader.",
        "Parse exactly one record at `reader`'s cursor, returning a `CsvRow` \
         (`fields`, advanced `reader`, `done`).",
        "csv::readRow(csv::parseStream(\"a,b\\nc,d\"))",
        vec![Implementation {
            params: vec![required("reader", &[], "CsvReader")],
            return_type: "CsvRow",
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(BODY_NEXT, "__csv_next"),
        }],
    );
}

#[cfg(test)]
mod tests {
    use super::super::registry;

    #[test]
    fn csv_reassembled_source_parses() {
        let pkg = registry()
            .get_package("csv")
            .expect("csv package registered");
        let source = pkg.get_mfb();
        assert!(source.contains("IMPORT collections"));
        assert!(source.contains("EXPORT TYPE CsvReader"));
        assert!(source.contains("FUNC __csv_parse("));
        assert!(source.contains("FUNC __csv_next(")); // readRow's irregular rewrite
                                                      // The reassembled package source is syntactically valid MFBASIC.
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-csv>"),
            "builtins/csv.mfb",
            &source,
        )
        .expect("reassembled csv source parses");
    }

    #[test]
    fn csv_members_expose_their_rewrite_targets() {
        let pkg = registry().get_package("csv").expect("csv package");
        let rewrite = |name: &str| {
            pkg.function(name).expect(name).implementations()[0]
                .body
                .rewrite_target()
        };
        assert_eq!(rewrite("parse"), Some("__csv_parse"));
        assert_eq!(rewrite("stringify"), Some("__csv_stringify"));
        assert_eq!(rewrite("parseStream"), Some("__csv_parseStream"));
        // The irregular pairing the public name cannot derive.
        assert_eq!(rewrite("readRow"), Some("__csv_next"));
    }
}
