//! `csv::parse` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:parse@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, RegistryFunction, RegistryPackage,
};

#[rustfmt::skip]
const FUNC_BODY: &str =
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

const INTRO: &str = r#"Parse UTF-8 CSV text into a grid of String cells."#;
const DESC: &str = r#"`csv::parse` scans `value` left to right and returns the resulting document as a
`List OF List OF String`: an ordered list of rows, each an ordered list of String
cells. Internally the text is decoded to its Unicode scalars in one pass
(`encoding::utf32Encode`) and scanned scalar by scalar, so the scanner never
splits a multi-byte code point or a `\r\n` pair incorrectly; each field is
accumulated in a scalar buffer and re-encoded to a String with
`encoding::utf32Decode`. Every structural CSV character (comma, quote, CR, LF) is
ASCII, so the resulting grid is byte-identical to a grapheme-based scan.

The dialect is RFC-4180-aligned. The field delimiter defaults to a comma (scalar
`44`) but can be overridden with the optional `delimiter` argument; the quote
character defaults to the double quote (`34`) but can be overridden with the
optional `quote` argument. Each must be a non-empty single character, and only its
first Unicode scalar is used. A record separator is a line feed (LF, `10`) or a
carriage-return/line-feed pair (CRLF, `13` then `10`) regardless of dialect; a
bare CR not followed by LF is ordinary data inside the current field. A field may
be wrapped in the quote character: the opening quote must be the first character
of the field, inside a quoted field a literal quote is written by doubling it, and
delimiters, CR, and LF are ordinary data. The closing quote must be immediately
followed by the delimiter, a record separator, or the end of input. Whitespace is
significant and never trimmed.

Cells are plain Strings with no type inference and no null: `42`, `true`, and an
empty field parse to the Strings `"42"`, `"true"`, and `""`. Callers that want
numbers convert explicitly with `toFloat` or `toInteger`. Rows are not required
to be rectangular; each row keeps whatever field count it had. A single trailing
record separator does not create an empty final row, so `"a\nb\n"` parses to two
rows, while two consecutive separators do produce an empty row in the middle.
Empty input parses to zero rows. There is no header concept — every parsed line
is an ordinary row, and cells are read positionally with `collections::get`.

The argument may also be supplied by the name `text`. `csv::parse` does not
mutate `value` and has no side effects."#;
const EX: &str = r#"Parse a two-column document with a quoted cell:

```
IMPORT csv

SUB main()
  LET doc AS List OF List OF String = csv::parse("name,age\nAda,36")
END SUB
```

Pass the argument by name:

```
IMPORT csv

SUB main()
  LET rows AS List OF List OF String = csv::parse(text := "a,b,c")
END SUB
```"#;

pub(super) fn add(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "parse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    aliases: &["text"],
                    ty: "String",
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "delimiter",
                    aliases: &[],
                    ty: "String",
                    default: DefaultValue::Fill {
                        type_name: "String",
                        expr: super::DEFAULT_DELIMITER,
                    },
                },
                Parameter {
                    name: "quote",
                    aliases: &[],
                    ty: "String",
                    default: DefaultValue::Fill {
                        type_name: "String",
                        expr: super::DEFAULT_QUOTE,
                    },
                },
            ],
            return_type: "List OF List OF String",
            errors: vec!["ErrInvalidFormat"],
            lowering: Lowering::Helper,
            body: Body::mfb(FUNC_BODY, "__csv_parse"),
        }],
    });
}
